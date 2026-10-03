use super::*;
use crate::store::schema;
use crate::test_support::attention_oracle::scan_effective_seat_attention;
use rusqlite::Connection;

struct InboxClock;
impl Clock for InboxClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> crate::protocol::time::MonoInstant {
        crate::protocol::time::MonoInstant(0)
    }
}
fn inbox_budget() -> CallBudget {
    CallBudget {
        deadline: crate::protocol::time::MonoInstant(100),
        cancellation: Default::default(),
    }
}

#[test]
fn inbox_token_detects_unprojected_receipt_in_examined_thread() {
    let db = fixture();
    let token = capture_inbox_token(&db, "i", "s").unwrap();
    db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',100,11); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',11,100,1,0,1,0); UPDATE host_instances SET decision_seq=11 WHERE id='i';").unwrap();
    assert_eq!(
        validate_inbox_token(&db, "i", "s", 1, token, 8, &inbox_budget(), &InboxClock).unwrap(),
        InboxValidation::Changed
    );
}

#[test]
fn inbox_token_scans_unrelated_publications_boundedly_then_completes() {
    let db = fixture();
    let mut token = capture_inbox_token(&db, "i", "s").unwrap();
    for i in 11..31 {
        db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES (?1,'i','actor',?1,zeroblob(32),'t',0,0,0,0,0,0,0,'sealed')",[format!("p{i}")]).unwrap();
        db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i',?1,'t',?2,'ordinary','body',100,?3)",rusqlite::params![format!("m{i}"),i-9,i]).unwrap();
        db.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,'t',?3,100,?4,0,0,0)",rusqlite::params![format!("p{i}"),format!("m{i}"),i,i-9]).unwrap();
    }
    db.execute("UPDATE host_instances SET decision_seq=30 WHERE id='i'", [])
        .unwrap();
    let mut saw_more = false;
    for _ in 0..20 {
        match validate_inbox_token(&db, "i", "s", 1, token, 8, &inbox_budget(), &InboxClock)
            .unwrap()
        {
            InboxValidation::Current { visited, .. } => {
                assert!(visited <= 8);
                assert!(saw_more);
                return;
            }
            InboxValidation::More {
                token: next,
                visited,
            } => {
                assert!(visited <= 8);
                saw_more = true;
                token = next;
            }
            InboxValidation::Changed => panic!("unaddressed publication changed inclusion"),
        }
    }
    panic!("unchanged finite publications did not complete");
}

#[test]
fn inbox_token_detects_unprojected_warning_and_revision_change() {
    let db = fixture();
    let token = capture_inbox_token(&db, "i", "s").unwrap();
    db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('v','t','s',1,'pending',10,0,300,300); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq,source_invitation_id) VALUES ('i','w','t',1,'warn','key','{}',300,11,'v'); INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('w',11,'t',0,'s','invitation','v'); UPDATE host_instances SET decision_seq=11 WHERE id='i';").unwrap();
    assert_eq!(
        validate_inbox_token(
            &db,
            "i",
            "s",
            1,
            token.clone(),
            8,
            &inbox_budget(),
            &InboxClock
        )
        .unwrap(),
        InboxValidation::Changed
    );
    assert!(matches!(
        validate_inbox_token(
            &db,
            "i",
            "s",
            0,
            token.clone(),
            8,
            &inbox_budget(),
            &InboxClock
        )
        .unwrap(),
        InboxValidation::Current { .. }
    ));
    db.execute("INSERT INTO filter_revisions(instance_id,scope_kind,scope_key,revision) VALUES ('i','inbox','s',1)",[]).unwrap();
    assert_eq!(
        validate_inbox_token(&db, "i", "s", 0, token, 8, &inbox_budget(), &InboxClock).unwrap(),
        InboxValidation::Changed
    );
}

#[test]
fn inbox_capture_roundtrips_in_bounded_typed_cursor() {
    use crate::protocol::pagination::{Cursor, CursorDirection, CursorScope};
    let db = fixture();
    let capture = capture_inbox_token(&db, "i", "s").unwrap();
    let cursor = Cursor {
        instance: "i".into(),
        scope: CursorScope::Inbox,
        scope_key: "s".into(),
        filter_digest: "f".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 1,
        high_water_ordinal: 2,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: Some(capture.to_cursor_state()),
        attention: None,
        binding: None,
    };
    let encoded = cursor.encode().unwrap();
    assert!(encoded.len() <= 1024);
    let decoded = Cursor::decode(&encoded).unwrap();
    assert_eq!(
        InboxCapture::from_cursor_state(decoded.inbox.unwrap()),
        capture
    );
    let mut wrong_scope = cursor;
    wrong_scope.scope = CursorScope::Directory;
    assert!(Cursor::decode(&wrong_scope.encode().unwrap()).is_err());
}

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    schema::initialize(&db, || crate::protocol::time::UtcMillis(0)).unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id, created_at, decision_seq) VALUES ('i', 0, 10);\
        INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('s', 'i', 'resolved', 'native', 1, 0);\
        INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('t', 'i', 'topic', 'goal', 0, 0);\
        INSERT INTO send_preparations(id, instance_id, operation_scope, operation_key, digest, thread_id, captured_membership_revision, captured_lifecycle_revision, captured_eligibility_revision, captured_timeline_revision, captured_config_revision, interval_high_water, recipient_high_water, status) VALUES ('p', 'i', 'actor', 'o', zeroblob(32), 't', 0, 0, 0, 0, 0, 0, 1, 'sealed');\
        INSERT INTO prepared_recipients(preparation_id, thread_id, seat_id, receipt_ordinal, frozen_duration_ms, eligible_at_snapshot) VALUES ('p', 't', 's', 1, 300, 0);\
    ").unwrap();
    db
}

#[test]
fn pending_receipt_for_human_is_not_an_ack_obligation() {
    let db = fixture();
    db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,0); INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES ('s',1,'p','b',0,'human','human','human','operator_human',1000,1000);").unwrap();
    let receipt = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(receipt.state, EffectiveReceiptState::NotRequired);
    assert!(receipt.ack_actor_seat_id.is_none());
}

#[test]
fn human_waiver_survives_agent_rebinding_without_erasing_ack_history() {
    let db = fixture();
    db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,0); INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('s',11,2,1100); INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES ('s',3,'p','b',0,'codex','agent','agent','cooperative_top_level',1500,1500);").unwrap();
    assert_eq!(
        effective_receipt(&db, "m", "s").unwrap().unwrap().state,
        EffectiveReceiptState::NotRequired
    );
    db.execute_batch("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p2','i','actor','o2',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed'); INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p2','t','s',1,300,0); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m2','t',3,'ordinary','new agent mail',1600,16); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p2','m2','t',16,1600,3,0,1,0);").unwrap();
    assert_eq!(
        effective_receipt(&db, "m2", "s").unwrap().unwrap().state,
        EffectiveReceiptState::Pending
    );
    db.execute_batch("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p3','i','actor','o3',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed'); INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,availability_provenance) VALUES ('p3','t','s',1,300,1,'operator_human'); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m3','t',2,'ordinary','legacy human mail',1400,14); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p3','m3','t',14,1400,2,0,1,0);").unwrap();
    assert_eq!(
        effective_receipt(&db, "m3", "s").unwrap().unwrap().state,
        EffectiveReceiptState::NotRequired
    );
    db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES ('m','s','acked','s',2,'operator_human',1400)", []).unwrap();
    let acked = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(acked.state, EffectiveReceiptState::Acknowledged);
    assert_eq!(acked.ack_observation.as_deref(), Some("operator_human"));
    db.execute(
        "UPDATE seats SET state='retired',retired_at=1700,retired_seq=17 WHERE id='s'",
        [],
    )
    .unwrap();
    assert_eq!(
        effective_receipt(&db, "m", "s").unwrap().unwrap().state,
        EffectiveReceiptState::Acknowledged
    );
    assert_eq!(
        effective_receipt(&db, "m3", "s").unwrap().unwrap().state,
        EffectiveReceiptState::NotRequired
    );
    assert_eq!(
        effective_receipt(&db, "m2", "s").unwrap().unwrap().state,
        EffectiveReceiptState::RecipientRetired
    );
}

#[test]
fn retained_baseline_holds_only_its_unowned_members_across_later_snapshots() {
    let db = fixture();
    db.execute_batch("\
        INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES ('g1','i','b',1,1,'inc',1,1,'published',0,0,0),('g2','i','b',1,2,'inc',2,2,'published',0,0,0);\
        INSERT INTO snapshot_targets(generation_id,target_id,terminal_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g1','old','term-old',1,1,'empty_shell','idle',0),('g2','old','term-old',1,2,'empty_shell','idle',1),('g2','new','term-new',1,2,'empty_shell','idle',1);\
        UPDATE host_instances SET host_boot='b',host_epoch=1,observation_sequence=2,active_snapshot_id='g2',recovery_baseline_generation_id='g1',baseline_hold_unclaimed=1 WHERE id='i';\
    ").unwrap();
    assert_eq!(
        effective_recovery_disposition(&db, "i", "old").unwrap(),
        EffectiveRecoveryDisposition::BaselineHeld
    );
    assert_eq!(
        effective_recovery_disposition(&db, "i", "new").unwrap(),
        EffectiveRecoveryDisposition::CreatedAfterBaseline
    );
    db.execute("INSERT INTO recovery_baseline_releases(instance_id,baseline_generation_id,target_id,decision_seq) VALUES ('i','g1','old',1)",[]).unwrap();
    assert_eq!(
        effective_recovery_disposition(&db, "i", "old").unwrap(),
        EffectiveRecoveryDisposition::Released
    );
    db.execute("UPDATE seats SET target_id='old' WHERE id='s'", [])
        .unwrap();
    assert_eq!(
        effective_recovery_disposition(&db, "i", "old").unwrap(),
        EffectiveRecoveryDisposition::Owned("s".into())
    );
}

#[test]
fn only_strictly_newer_same_boot_target_override_beats_publication() {
    let db = fixture();
    db.execute_batch("\
        INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES ('g','i','b',1,5,'inc',1,1,'published',0,0,0);\
        INSERT INTO snapshot_targets(generation_id,target_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g','p',4,5,'occupied','idle',0);\
        UPDATE host_instances SET host_boot='b',host_epoch=1,observation_sequence=5,active_snapshot_id='g' WHERE id='i';\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance) VALUES ('i','p','b',1,9,4,1,'fresh');\
    ").unwrap();
    assert_eq!(
        effective_observation(&db, "i", "p")
            .unwrap()
            .unwrap()
            .structural_generation,
        4
    );
    db.execute("UPDATE observed_targets SET observation_sequence=6 WHERE instance_id='i' AND target_id='p'",[]).unwrap();
    assert_eq!(
        effective_observation(&db, "i", "p")
            .unwrap()
            .unwrap()
            .structural_generation,
        9
    );
    db.execute("UPDATE observed_targets SET host_boot='other',observation_sequence=7 WHERE instance_id='i' AND target_id='p'",[]).unwrap();
    assert_eq!(
        effective_observation(&db, "i", "p")
            .unwrap()
            .unwrap()
            .structural_generation,
        4
    );
}

#[test]
fn host_invalidation_immediately_denies_published_target_until_new_publication() {
    let db = fixture();
    db.execute_batch("INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,published_invalidation_revision,admission_sequence,created_at) VALUES ('g','i','b',1,1,'inc',1,1,'published',0,0,0,1,0); INSERT INTO snapshot_targets(generation_id,target_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g','p',1,1,'occupied','idle',0); UPDATE host_instances SET host_boot='b',host_epoch=1,observation_sequence=1,active_snapshot_id='g',observation_admission_sequence=1,observation_decided_sequence=1 WHERE id='i';").unwrap();
    assert!(effective_observation(&db, "i", "p").unwrap().is_some());
    db.execute("UPDATE host_instances SET invalidation_revision=1,observation_admission_sequence=2,observation_decided_sequence=2 WHERE id='i'",[]).unwrap();
    assert!(effective_observation(&db, "i", "p").unwrap().is_none());
    db.execute(
        "UPDATE snapshot_generations SET published_invalidation_revision=1 WHERE id='g'",
        [],
    )
    .unwrap();
    assert!(effective_observation(&db, "i", "p").unwrap().is_some());
}

#[test]
fn new_unregistered_seat_starts_in_a_real_unavailability_episode() {
    let db = fixture();
    let episode: i64 = db
        .query_row(
            "SELECT unavailability_episode FROM seats WHERE id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(episode, 1);
    assert!(
        db.execute("UPDATE seats SET unavailability_episode=0 WHERE id='s'", [])
            .is_err()
    );
}

#[test]
fn unpublished_staging_is_invisible_and_first_later_anchor_is_immutable() {
    let db = fixture();
    assert!(effective_receipt(&db, "m", "s").unwrap().is_none());
    db.execute_batch("\
        INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at, decision_seq) VALUES ('i','m', 't', 1, 'ordinary', 'body', 1000, 10);\
        INSERT INTO send_manifests(instance_id,preparation_id, message_id, thread_id, decision_seq, decision_at, base_sequence, interval_high_water, recipient_count, warning_count) VALUES ('i','p', 'm', 't', 10, 1000, 1, 0, 1, 0);\
    ").unwrap();
    assert!(
        db.execute(
            "UPDATE send_manifests SET instance_id='other' WHERE message_id='m'",
            []
        )
        .is_err()
    );
    let before = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(before.thread_id, "t");
    assert_eq!(before.frozen_duration_ms, 300);
    assert_eq!(before.source, ReceiptSource::Manifest);
    assert_eq!(before.warning_message_id, None);
    assert_eq!(before.available_at, None);
    assert_eq!(before.deadline_at, None);
    db.execute_batch("\
        INSERT INTO seat_availability(seat_id, decision_seq, decision_at, binding_generation, observation_provenance) VALUES ('s', 11, 1200, 1, 'verified');\
        INSERT INTO seat_availability(seat_id, decision_seq, decision_at, binding_generation, observation_provenance) VALUES ('s', 12, 500, 1, 'verified');\
    ").unwrap();
    let after = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(after.available_at, Some(1200));
    assert_eq!(after.deadline_at, Some(1500));
    assert_eq!(after.state, EffectiveReceiptState::Pending);
}

#[test]
fn sparse_ack_and_retirement_preserve_logical_receipt_provenance() {
    let db = fixture();
    db.execute_batch("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at, decision_seq) VALUES ('i','m', 't', 1, 'ordinary', 'body', 1000, 10); INSERT INTO send_manifests(instance_id,preparation_id, message_id, thread_id, decision_seq, decision_at, base_sequence, interval_high_water, recipient_count, warning_count) VALUES ('i','p', 'm', 't', 10, 1000, 1, 0, 1, 0); INSERT INTO receipt_state(message_id, seat_id, state, ack_actor_seat_id, ack_generation, ack_observation, acked_at, warning_message_id) VALUES ('m', 's', 'acked', 's', 1, 'proof', 1300, 'warning-id'); UPDATE seats SET state='retired', retired_at=1400, retired_seq=13 WHERE id='s';").unwrap();
    let receipt = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(receipt.state, EffectiveReceiptState::Acknowledged);
    assert_eq!(receipt.ack_actor_seat_id.as_deref(), Some("s"));
    assert_eq!(receipt.warning_message_id.as_deref(), Some("warning-id"));
    assert_eq!(receipt.retired_at, Some(1400));
}

#[test]
fn bounded_scan_discovers_unprojected_receipt_and_deduplicates_projection() {
    let db = fixture();
    db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,0); INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m','t','s','pending',300);").unwrap();
    let first = scan_effective_receipts(&db, &ReceiptScanScope::Seat("s".into()), None, 1).unwrap();
    assert_eq!(first.visited, 1);
    assert!(first.has_more);
    let second = scan_effective_receipts(
        &db,
        &ReceiptScanScope::Seat("s".into()),
        Some(first.position),
        1,
    )
    .unwrap();
    assert_eq!(second.visited, 1);
    assert!(!second.has_more);
    let all: Vec<_> = first.items.into_iter().chain(second.items).collect();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].source, ReceiptSource::Manifest);
    let thread =
        scan_effective_receipts(&db, &ReceiptScanScope::Thread("t".into()), None, 3).unwrap();
    assert_eq!(thread.items.len(), 1);
    assert_eq!(thread.items[0].message_id, "m");
    assert!(!thread.has_more);
}

#[test]
fn overdue_manifest_receipt_gets_sparse_marker_and_one_warning_atomically() {
    use crate::ports::TimeBasis;
    use crate::protocol::{
        authority::ObligationRef,
        ids::{MessageId, SeatId},
        time::UtcMillis,
    };
    let mut db = fixture();
    db.execute_batch("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at, decision_seq) VALUES ('i','m', 't', 1, 'ordinary', 'body', 1000, 10); UPDATE threads SET next_sequence=2 WHERE id='t'; INSERT INTO send_manifests(instance_id,preparation_id, message_id, thread_id, decision_seq, decision_at, base_sequence, interval_high_water, recipient_count, warning_count) VALUES ('i','p', 'm', 't', 10, 1000, 1, 0, 1, 0); INSERT INTO seat_availability(seat_id, decision_seq, decision_at, binding_generation, observation_provenance) VALUES ('s', 11, 1200, 1, 'verified'); UPDATE host_instances SET decision_seq=11 WHERE id='i';").unwrap();
    let obligation = ObligationRef::Receipt {
        message: MessageId::new("m"),
        seat: SeatId::new("s"),
    };
    let tx = db.transaction().unwrap();
    let warning =
        schema::record_overdue_if_pending(&tx, &obligation, &TimeBasis::Decision, UtcMillis(1500))
            .unwrap();
    assert!(warning.inserted);
    tx.commit().unwrap();
    let receipt = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(
        receipt.warning_message_id.as_deref(),
        warning.warning.as_ref().map(|v| v.as_str())
    );
    assert_eq!(receipt.source, ReceiptSource::Manifest);
    assert_eq!(
        db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM warning_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn persisted_overdue_transition_stays_actionable_until_offered_after_human_check_in() {
    use crate::ports::TimeBasis;
    use crate::protocol::{
        authority::ObligationRef,
        ids::{MessageId, SeatId},
    };
    let mut db = fixture();
    db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); UPDATE threads SET next_sequence=2 WHERE id='t'; INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,0); INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('s',11,1200,1,'verified'); UPDATE host_instances SET decision_seq=11 WHERE id='i';").unwrap();
    let tx = db.transaction().unwrap();
    let outcome = schema::record_overdue_if_pending(
        &tx,
        &ObligationRef::Receipt {
            message: MessageId::new("m"),
            seat: SeatId::new("s"),
        },
        &TimeBasis::Decision,
        UtcMillis(1500),
    )
    .unwrap();
    tx.commit().unwrap();
    let warning_id = outcome.warning.unwrap();
    let warning = effective_warning_by_id(&db, warning_id.as_str())
        .unwrap()
        .unwrap();
    assert!(warning_condition_actionable(&db, &warning).unwrap());
    db.execute_batch("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES ('s',1,'p','b',0,'human','human','human','operator_human',1600,1600); INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('s',12,1,1600);").unwrap();
    assert!(warning_condition_actionable(&db, &warning).unwrap());
    assert!(
        effective_warning_by_id(&db, warning_id.as_str())
            .unwrap()
            .is_some()
    );
}

#[test]
fn published_warning_keeps_sequence_and_historical_recipient_before_projection() {
    let db = fixture();
    db.execute_batch("\
        INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('joined', 'i', 'resolved', 'native', 1, 0);\
        INSERT INTO membership_intervals(thread_id, seat_id, episode, joined_seq, left_seq) VALUES ('t', 'joined', 1, 4, 12);\
        INSERT INTO prepared_unavailable_warnings(preparation_id, warning_key, warning_id, affected_seat_id, unavailability_episode, warning_offset, event_json) VALUES ('p', 'key', 'warn-id', 's', 1, 1, '{}');\
    ").unwrap();
    assert!(effective_warning_by_id(&db, "warn-id").unwrap().is_none());
    db.execute_batch("\
        INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at, decision_seq) VALUES ('i','m', 't', 1, 'ordinary', 'body', 1000, 10);\
        INSERT INTO send_manifests(instance_id,preparation_id, message_id, thread_id, decision_seq, decision_at, base_sequence, interval_high_water, recipient_count, warning_count) VALUES ('i','p', 'm', 't', 10, 1000, 1, 1, 1, 1);\
    ").unwrap();
    let warning = effective_warning_by_id(&db, "warn-id").unwrap().unwrap();
    assert_eq!(warning.sequence, 2);
    assert_eq!(warning.event_seq, 10);
    assert_eq!(warning.source_message_id.as_deref(), Some("m"));
    assert!(is_warning_recipient(&db, "warn-id", "joined").unwrap());
    assert!(is_warning_recipient(&db, "warn-id", "s").unwrap());
    assert!(!is_warning_recipient(&db, "warn-id", "absent").unwrap());
}

#[test]
fn timeline_slice_finds_logical_warning_before_and_after_projection() {
    let db = fixture();
    db.execute_batch("INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','key','warn-id','s',1,1,'{}'); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,1); UPDATE threads SET next_sequence=3 WHERE id='t';").unwrap();
    let first = scan_effective_timeline(&db, "t", None, 1).unwrap();
    assert_eq!(first.entries.len(), 1);
    assert!(first.has_more);
    let second = scan_effective_timeline(&db, "t", Some(first.position), 1).unwrap();
    assert!(
        matches!(&second.entries[0],EffectiveTimelineEntry::PublishedWarning(w) if w.id=="warn-id" && w.sequence==2)
    );
    assert!(!second.has_more);
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq,event_offset) VALUES ('i','warn-id','t',2,'warn','key','{}',1000,10,1)",[]).unwrap();
    let projected = scan_effective_timeline(&db, "t", None, 2).unwrap();
    assert_eq!(projected.entries.len(), 2);
    assert!(
        matches!(&projected.entries[1],EffectiveTimelineEntry::Physical{ id,sequence,.. } if id=="warn-id" && *sequence==2)
    );
}

#[test]
fn global_logical_scan_orders_direct_and_manifest_events_before_projection() {
    let mut db = fixture();
    db.execute("INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','key','logical-warning','s',1,1,'{}')",[]).unwrap();
    assert!(
        scan_global_logical_candidates(&db, "i", GlobalLogicalKinds::Warnings, None, 2)
            .unwrap()
            .candidates
            .is_empty()
    );
    db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',100,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,100,1,0,1,1); UPDATE threads SET next_sequence=3;").unwrap();
    let tx = db.transaction().unwrap();
    let seq = schema::next_decision_seq(&tx, "i").unwrap();
    schema::append_event_once_with_decision_seq(
        &tx,
        schema::EventInput {
            thread: &crate::protocol::ids::ThreadId::new("t"),
            key: "direct-warning",
            kind: "warn",
            payload_json: "{}",
            decision_at: crate::protocol::time::UtcMillis(101),
            source_message: None,
            source_invitation: None,
        },
        seq,
    )
    .unwrap();
    tx.commit().unwrap();
    let first = scan_global_logical_candidates(&db, "i", GlobalLogicalKinds::All, None, 2).unwrap();
    assert_eq!(first.visited, 2);
    assert_eq!(
        first
            .candidates
            .iter()
            .map(|c| (c.decision_seq, c.event_offset))
            .collect::<Vec<_>>(),
        vec![(10, 0)]
    );
    let second =
        scan_global_logical_candidates(&db, "i", GlobalLogicalKinds::All, Some(first.position), 2)
            .unwrap();
    assert_eq!(second.candidates[0].id, "logical-warning");
    assert_eq!(
        (
            second.candidates[0].decision_seq,
            second.candidates[0].event_offset
        ),
        (10, 1)
    );
    let third =
        scan_global_logical_candidates(&db, "i", GlobalLogicalKinds::All, Some(second.position), 2)
            .unwrap();
    assert_eq!(
        (
            third.candidates[0].decision_seq,
            third.candidates[0].event_offset
        ),
        (11, 0)
    );
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq,event_offset) VALUES ('i','logical-warning','t',2,'warn','key','{}',100,10,1)",[]).unwrap();
    let warnings =
        scan_global_logical_candidates(&db, "i", GlobalLogicalKinds::Warnings, None, 100).unwrap();
    assert_eq!(
        warnings
            .candidates
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        vec!["logical-warning", third.candidates[0].id.as_str()]
    );
}

#[test]
fn global_logical_queries_use_instance_and_warning_partial_indexes() {
    let db = fixture();
    for (sql, index) in [
        (
            "EXPLAIN QUERY PLAN SELECT id FROM messages WHERE instance_id='i' AND kind='ordinary' AND (decision_seq,event_offset)>(0,-1) AND decision_seq<=10 ORDER BY decision_seq,event_offset LIMIT 1",
            "messages_instance_ordinary_logical",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT id FROM messages WHERE instance_id='i' AND kind='warn' AND (decision_seq,event_offset)>(0,-1) AND decision_seq<=10 ORDER BY decision_seq,event_offset LIMIT 1",
            "messages_instance_warning_logical",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT preparation_id FROM send_manifests WHERE instance_id='i' AND decision_seq>0 AND decision_seq<=10 AND warning_count>0 ORDER BY decision_seq LIMIT 1",
            "send_manifests_warning_decision",
        ),
    ] {
        let plans: Vec<String> = db
            .prepare(sql)
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            plans.iter().any(|p| p.contains(index)),
            "{index}: {plans:?}"
        );
    }
}

#[test]
fn warning_slice_counts_unprojected_historical_recipient_once() {
    let db = fixture();
    db.execute_batch("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s',1,5); INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','key','warn-id','s',1,1,'{}'); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,1,1,1); UPDATE threads SET next_sequence=3 WHERE id='t';").unwrap();
    let slice = scan_effective_warnings_for_seat(&db, "t", "s", None, 2).unwrap();
    // Digest fix3: only the warning position is visited, not the ordinary
    // message at sequence 1.
    assert_eq!(slice.visited, 1);
    assert_eq!(slice.warnings.len(), 1);
    assert_eq!(slice.warnings[0].id, "warn-id");
    assert!(!slice.has_more);
}

#[test]
fn warning_recipient_page_counts_intervals_and_direct_affected_once() {
    let db = fixture();
    db.execute_batch("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('joined','i','resolved','native',1,0); INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','joined',1,5); INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','key','warn-id','joined',1,1,'{}'); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,1,1,1);").unwrap();
    let first = scan_effective_warning_recipients(&db, "warn-id", None, 1).unwrap();
    assert_eq!(first.visited, 1);
    assert_eq!(first.seats, vec!["joined"]);
    assert!(first.has_more);
    let second =
        scan_effective_warning_recipients(&db, "warn-id", Some(first.position), 1).unwrap();
    assert_eq!(second.visited, 1);
    assert!(second.seats.is_empty());
    assert!(!second.has_more);
}

#[test]
fn warning_recipient_page_includes_affected_seat_without_membership() {
    let db = fixture();
    db.execute_batch("INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','key','warn-id','s',1,1,'{}'); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,1);").unwrap();
    let page = scan_effective_warning_recipients(&db, "warn-id", None, 1).unwrap();
    assert_eq!(page.seats, vec!["s"]);
    assert_eq!(page.visited, 1);
    assert!(!page.has_more);
}

#[test]
fn physical_warning_with_only_recipient_ledger_pages_its_historical_seats() {
    let db = fixture();
    db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES ('i','warn','t',1,'warn','legacy','{}',100,10); INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES ('warn','s',1);").unwrap();
    let page = scan_effective_warning_recipients(&db, "warn", None, 1).unwrap();
    assert_eq!(page.seats, vec!["s"]);
    assert_eq!(page.visited, 1);
    assert!(!page.has_more);
}

#[test]
fn seat_attention_finds_unprojected_receipt_and_warning_with_bounded_progress() {
    let db = fixture();
    db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('v','t','s',1,'pending',10,0,300,300); INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','key','warning','s',1,1,'{}'); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,1);").unwrap();
    let mut position = None;
    let mut total_visited = 0;
    loop {
        let slice = scan_effective_seat_attention(&db, "s", position, 1).unwrap();
        assert_eq!(slice.visited, 1);
        total_visited += usize::from(slice.visited);
        if !slice.has_more {
            let attention = slice.attention.unwrap();
            assert!(attention.has_pending_invitation);
            assert!(attention.has_pending_receipt);
            assert_eq!(attention.latest_warning_seq, Some(10));
            db.execute_batch("UPDATE seats SET unavailability_open=0 WHERE id='s'; UPDATE host_instances SET decision_seq=11 WHERE id='i';").unwrap();
            let settled = scan_effective_seat_attention(&db, "s", None, 100)
                .unwrap()
                .attention
                .unwrap();
            assert_eq!(settled.latest_warning_seq, None);
            break;
        }
        assert!(slice.attention.is_none());
        position = Some(slice.position);
        assert!(total_visited < 10);
    }
}

#[test]
fn seat_attention_ignores_settled_warning_without_erasing_history() {
    let db = fixture();
    db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('v','t','s',1,'pending',10,0,300,300); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq,source_invitation_id) VALUES ('i','w','t',1,'warn','key','{}',300,10,'v'); INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('w',10,'t',0,'s','invitation','v');").unwrap();
    let before = scan_effective_seat_attention(&db, "s", None, 100)
        .unwrap()
        .attention
        .unwrap();
    assert_eq!(before.latest_warning_seq, Some(10));
    db.execute_batch("UPDATE invitations SET state='accepted',accepted_at=301,accepted_actor_seat_id='s',accepted_generation=1,accepted_observation='proof' WHERE id='v'; UPDATE host_instances SET decision_seq=11 WHERE id='i';").unwrap();
    assert!(is_warning_recipient(&db, "w", "s").unwrap());
    let after = scan_effective_seat_attention(&db, "s", None, 100)
        .unwrap()
        .attention
        .unwrap();
    assert_eq!(after.latest_warning_seq, None);
}

#[test]
fn canonical_unavailable_warning_key_is_stable_and_only_published_rows_count() {
    let db = fixture();
    let key = UnavailableWarningKey {
        instance: "i".into(),
        thread_id: "t".into(),
        affected_seat_id: "s".into(),
        unavailability_episode: 3,
    };
    let encoded = canonical_warning_key(&key).unwrap();
    let id = canonical_warning_id(&key).unwrap();
    assert!(id.starts_with('w'));
    assert_eq!(id, canonical_warning_id(&key).unwrap());
    assert_ne!(
        id,
        canonical_warning_id(&UnavailableWarningKey {
            unavailability_episode: 4,
            ..key.clone()
        })
        .unwrap()
    );
    db.execute("INSERT INTO prepared_unavailable_warnings(preparation_id, warning_key, warning_id, affected_seat_id, unavailability_episode, warning_offset, event_json) VALUES ('p', ?1, ?2, 's', 3, 1, '{}')", rusqlite::params![encoded, id]).unwrap();
    assert!(effective_warning_by_key(&db, &key).unwrap().is_none());
    db.execute_batch("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at, decision_seq) VALUES ('i','m', 't', 1, 'ordinary', 'body', 1000, 10); INSERT INTO send_manifests(instance_id,preparation_id, message_id, thread_id, decision_seq, decision_at, base_sequence, interval_high_water, recipient_count, warning_count) VALUES ('i','p', 'm', 't', 10, 1000, 1, 0, 1, 1);").unwrap();
    assert_eq!(effective_warning_by_key(&db, &key).unwrap().unwrap().id, id);
    let legacy_key = UnavailableWarningKey {
        unavailability_episode: 4,
        ..key
    };
    let compact_id = canonical_warning_id(&legacy_key).unwrap();
    let legacy_id = format!("warning-{}", &compact_id[1..]);
    db.execute(
        "INSERT INTO prepared_unavailable_warnings(preparation_id, warning_key, warning_id, affected_seat_id, unavailability_episode, warning_offset, event_json) VALUES ('p', ?1, ?2, 's', 4, 2, '{}')",
        rusqlite::params![canonical_warning_key(&legacy_key).unwrap(), legacy_id],
    )
    .unwrap();
    assert_eq!(
        effective_warning_by_key(&db, &legacy_key)
            .unwrap()
            .unwrap()
            .id,
        legacy_id
    );
}

#[test]
fn prepared_recipient_has_global_immutable_ordinal_for_seat_keysets() {
    let db = fixture();
    let ordinal: i64 = db
        .query_row(
            "SELECT ordinal FROM prepared_recipients WHERE preparation_id='p' AND seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ordinal > 0);
    let index_exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='prepared_recipients_seat_ordinal')", [], |r| r.get(0)).unwrap();
    assert!(index_exists);
    let thread: String = db
        .query_row(
            "SELECT thread_id FROM prepared_recipients WHERE ordinal=?1",
            [ordinal],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(thread, "t");
    let scoped_index: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='prepared_recipients_seat_thread_ordinal')", [], |r| r.get(0)).unwrap();
    assert!(scoped_index);
    assert!(
        db.execute(
            "UPDATE prepared_recipients SET ordinal=ordinal+1 WHERE preparation_id='p'",
            []
        )
        .is_err()
    );
}

/// The timeline scan run to completion in `page`-sized slices.
fn full_timeline(
    db: &Connection,
    page: u16,
    scan: fn(
        &Connection,
        &str,
        Option<TimelinePosition>,
        u16,
    ) -> Result<EffectiveTimelineSlice, ApiError>,
) -> (Vec<EffectiveTimelineEntry>, u64) {
    let mut entries = Vec::new();
    let mut visited = 0u64;
    let mut position = None;
    loop {
        let slice = scan(db, "t", position, page).unwrap();
        visited += u64::from(slice.visited);
        entries.extend(slice.entries);
        if !slice.has_more {
            return (entries, visited);
        }
        position = Some(slice.position);
    }
}

// Digest fix3: the warning-only walk that check-in's inbox, warning count and
// warnings page now use returns exactly the warning entries of the full
// timeline (same order, same identity, projection preferred at a position),
// at every page size, without visiting ordinary messages. The fixture mixes a
// direct warn event, a two-warning manifest before and after one of its
// warnings is projected, a manifest ending at the high water and ordinary
// messages between them. Kills: dropping the manifest stream (unprojected
// warnings vanish), a covering-manifest test that skips the second warning of
// a multi-warning manifest (`base+count>after` off by one), a walk that
// ignores the frozen high water (the next-manifest seek's `base_sequence<high`
// bound and the final `<= high` filter both dropped; each covers the other,
// so dropping only one of them survives), and a walk that still steps through
// ordinary positions (`visited` would equal the timeline's).
#[test]
fn warning_timeline_equals_the_full_timeline_restricted_to_warnings() {
    let db = fixture();
    db.execute_batch("\
        INSERT INTO send_preparations(id, instance_id, operation_scope, operation_key, digest, thread_id, captured_membership_revision, captured_lifecycle_revision, captured_eligibility_revision, captured_timeline_revision, captured_config_revision, interval_high_water, recipient_high_water, status) VALUES ('p2', 'i', 'actor', 'o2', zeroblob(32), 't', 0, 0, 0, 0, 0, 0, 0, 'sealed'), ('p3', 'i', 'actor', 'o3', zeroblob(32), 't', 0, 0, 0, 0, 0, 0, 0, 'sealed');\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','o1','t',1,'ordinary','b',0,1),('i','o2','t',2,'ordinary','b',0,2);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('i','direct','t',3,'warn','{}',0,3,0);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',4,'ordinary','b',0,4),('i','o3','t',7,'ordinary','b',0,5),('i','o4','t',8,'ordinary','b',0,6),('i','m3','t',9,'ordinary','b',0,7);\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','k1','w1','s',1,1,'{}'),('p','k2','w2','s',2,2,'{}'),('p3','k3','w3','s',3,1,'{}');\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',4,0,4,0,1,2),('i','p3','m3','t',7,0,9,0,0,1);\
        UPDATE threads SET next_sequence=11 WHERE id='t';\
    ").unwrap();
    let warn_only = |entries: Vec<EffectiveTimelineEntry>| -> Vec<EffectiveTimelineEntry> {
        entries
            .into_iter()
            .filter(|entry| match entry {
                EffectiveTimelineEntry::Physical { kind, .. } => kind == "warn",
                EffectiveTimelineEntry::PublishedWarning(_) => true,
            })
            .collect()
    };
    let ids = |entries: &[EffectiveTimelineEntry]| -> Vec<(String, i64)> {
        entries
            .iter()
            .map(|entry| match entry {
                EffectiveTimelineEntry::Physical { id, sequence, .. } => (id.clone(), *sequence),
                EffectiveTimelineEntry::PublishedWarning(w) => (w.id.clone(), w.sequence),
            })
            .collect()
    };
    for projected in [false, true] {
        if projected {
            db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq,event_offset) VALUES ('i','w2','t',6,'warn','k2','{}',0,4,2)",[]).unwrap();
        }
        let (full, full_visited) = full_timeline(&db, 100, scan_effective_timeline);
        let expected = warn_only(full);
        assert_eq!(
            ids(&expected),
            [
                ("direct".to_string(), 3),
                ("w1".into(), 5),
                ("w2".into(), 6),
                ("w3".into(), 10)
            ]
        );
        assert_eq!(full_visited, 10);
        for page in [1u16, 2, 3, 100] {
            let (warnings, visited) = full_timeline(&db, page, scan_effective_warning_timeline);
            assert_eq!(warnings, expected, "page {page}, projected {projected}");
            assert_eq!(visited, 4, "page {page}, projected {projected}");
        }
        // A frozen high water below the last manifest's warning excludes it.
        let bounded = scan_effective_warning_timeline(
            &db,
            "t",
            Some(TimelinePosition {
                after_sequence: 5,
                high_water_sequence: 9,
            }),
            100,
        )
        .unwrap();
        assert_eq!(ids(&bounded.entries), [("w2".to_string(), 6)]);
        assert!(!bounded.has_more);
        assert_eq!(bounded.position.after_sequence, 9);
    }
}
