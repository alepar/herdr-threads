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
    populate_fixture(&db);
    db
}

fn populate_fixture(db: &Connection) {
    db.execute_batch("\
        INSERT INTO host_instances(id, created_at, decision_seq) VALUES ('i', 0, 10);\
        INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('s', 'i', 'resolved', 'native', 1, 0);\
        INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('t', 'i', 'topic', 'goal', 0, 0);\
        INSERT INTO send_preparations(id, instance_id, operation_scope, operation_key, digest, thread_id, captured_membership_revision, captured_lifecycle_revision, captured_eligibility_revision, captured_timeline_revision, captured_config_revision, interval_high_water, recipient_high_water, status) VALUES ('p', 'i', 'actor', 'o', zeroblob(32), 't', 0, 0, 0, 0, 0, 0, 1, 'sealed');\
        INSERT INTO prepared_recipients(preparation_id, thread_id, seat_id, receipt_ordinal, frozen_duration_ms, eligible_at_snapshot) VALUES ('p', 't', 's', 1, 300, 0);\
    ").unwrap();
}

fn v28_fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    for migration in [
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
        include_str!("../../migrations/0027_harness_adapters.sql"),
        include_str!("../../migrations/0028_handoff_topology.sql"),
    ] {
        db.execute_batch(migration).unwrap();
    }
    db.pragma_update(None, "user_version", 28).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    populate_fixture(&db);
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
fn unpublished_staging_is_invisible_and_send_deadline_survives_availability() {
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
    assert_eq!(before.deadline_at, Some(1300));
    db.execute_batch("\
        INSERT INTO seat_availability(seat_id, decision_seq, decision_at, binding_generation, observation_provenance) VALUES ('s', 11, 1200, 1, 'verified');\
        INSERT INTO seat_availability(seat_id, decision_seq, decision_at, binding_generation, observation_provenance) VALUES ('s', 12, 500, 1, 'verified');\
    ").unwrap();
    let after = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(after.available_at, Some(1200));
    assert_eq!(after.deadline_at, Some(1300));
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

#[test]
fn manifest_preserves_preexisting_running_deadline() {
    let db = fixture();
    db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,0); INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) VALUES ('m','s','pending',1200,1500);").unwrap();
    let receipt = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(receipt.deadline_at, Some(1500));
    assert_eq!(receipt.available_at, Some(1200));
}

#[test]
fn historical_sparse_running_deadline_survives_semantics_cutover_and_restart() {
    let db = v28_fixture();
    db.execute_batch("UPDATE host_instances SET decision_seq=11 WHERE id='i'; INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,0); INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES('s',11,1200,1,'cooperative_top_level');").unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    assert_eq!(
        effective_receipt(&db, "m", "s")
            .unwrap()
            .unwrap()
            .deadline_at,
        Some(1500)
    );
    db.execute("UPDATE host_instances SET decision_seq=99 WHERE id='i'", [])
        .unwrap();
    schema::initialize(&db, || UtcMillis(999999)).unwrap();
    assert_eq!(
        effective_receipt(&db, "m", "s")
            .unwrap()
            .unwrap()
            .deadline_at,
        Some(1500)
    );
    let cutoff: i64 = db
        .query_row(
            "SELECT historical_decision_seq FROM receipt_send_time_cutovers WHERE instance_id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cutoff, 11);
}

#[test]
fn historical_dormant_sparse_deadline_does_not_restart_after_cutover() {
    let db = fixture();
    db.execute_batch("INSERT INTO receipt_send_time_cutovers(instance_id,historical_decision_seq) VALUES('i',10); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m','t',1,'ordinary','body',1000,10); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i','p','m','t',10,1000,1,0,1,0);").unwrap();
    assert_eq!(
        effective_receipt(&db, "m", "s")
            .unwrap()
            .unwrap()
            .deadline_at,
        Some(1300)
    );
    db.execute_batch("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES('s',11,1200,1,'cooperative_top_level');").unwrap();
    let receipt = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(receipt.available_at, Some(1200));
    assert_eq!(receipt.deadline_at, Some(1300));
}

fn receipt_upgrade_snapshot(db: &Connection, table: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = db
        .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
        .unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

struct UpgradeClock;
impl Clock for UpgradeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(5000)
    }
    fn monotonic_now(&self) -> crate::protocol::time::MonoInstant {
        crate::protocol::time::MonoInstant(0)
    }
}

fn dormant_upgrade_fixture(physical: usize, manifests: usize) -> (std::path::PathBuf, Connection) {
    let db = v28_fixture();
    for index in 0..physical + manifests {
        let message = format!("old{index:04}");
        let sequence = index as i64 + 1;
        db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i',?1,'t',?2,'ordinary','retained body',1000,?2)", rusqlite::params![message, sequence]).unwrap();
        if index < physical {
            db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES(?1,'t','s','pending',300)", [&message]).unwrap();
        } else {
            let preparation = format!("oldp{index:04}");
            db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES(?1,'i','actor',?1,zeroblob(32),'t',0,0,0,0,0,0,1,'sealed')", [&preparation]).unwrap();
            db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES(?1,'t','s',1,300,0)", [&preparation]).unwrap();
            db.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i',?1,?2,'t',?3,1000,?3,0,1,0)", rusqlite::params![preparation, message, sequence]).unwrap();
        }
    }
    db.execute(
        "UPDATE host_instances SET decision_seq=?1",
        [(physical + manifests + 10) as i64],
    )
    .unwrap();
    db.execute(
        "UPDATE threads SET next_sequence=?1",
        [(physical + manifests + 1) as i64],
    )
    .unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        28
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    let path = std::env::temp_dir().join(format!("dormant-upgrade-{}.db", uuid::Uuid::new_v4()));
    db.execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    drop(db);
    let reopened = Connection::open(&path).unwrap();
    reopened.pragma_update(None, "foreign_keys", true).unwrap();
    assert_eq!(
        reopened
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        28
    );
    (path, reopened)
}

#[test]
fn dormant_upgrade_actual_v31_backfill32_reopens_and_rolls_back_without_recapture() {
    for foreign_keys in [false, true] {
        let (path, db) = dormant_upgrade_fixture(1, 1);
        // Replay immutable prefixes on the actual populated v28 file; do not
        // initialize latest and rewind its version to manufacture a precursor.
        db.pragma_update(None, "foreign_keys", false).unwrap();
        for (version, migration) in [
            (29, include_str!("../../migrations/0029_direct_threads.sql")),
            (
                30,
                include_str!("../../migrations/0030_automatic_handoff.sql"),
            ),
            (31, include_str!("../../migrations/0031_seat_wake_on.sql")),
            (
                32,
                include_str!("../../migrations/0032_session_bindings.sql"),
            ),
            (
                33,
                include_str!("../../migrations/0033_archival_scan_scope.sql"),
            ),
            (
                34,
                include_str!("../../migrations/0034_binding_delivery_state.sql"),
            ),
            (
                35,
                include_str!("../../migrations/0035_binding_attention_frontiers.sql"),
            ),
            (36, include_str!("../../migrations/0036_in_turn_offer.sql")),
            (
                37,
                include_str!("../../migrations/0037_invitation_confirmation.sql"),
            ),
            (
                38,
                include_str!("../../migrations/0038_receipt_send_time.sql"),
            ),
            (
                39,
                include_str!("../../migrations/0039_handoff_body_bounds.sql"),
            ),
        ] {
            db.execute_batch("BEGIN IMMEDIATE").unwrap();
            db.execute_batch(migration).unwrap();
            db.pragma_update(None, "user_version", version).unwrap();
            db.execute_batch("COMMIT").unwrap();
        }
        db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES('retained-invitation','t','s',1,'pending',1,100,50,150,'require_ack','s'); UPDATE host_instances SET decision_seq=99 WHERE id='i';").unwrap();
        drop(db);
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", foreign_keys)
            .unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            39
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        let tables = [
            "receipts",
            "receipt_state",
            "messages",
            "send_preparations",
            "prepared_recipients",
            "send_manifests",
            "invitations",
            "receipt_send_time_cutovers",
        ];
        let before: Vec<_> = tables
            .iter()
            .map(|table| receipt_upgrade_snapshot(&db, table))
            .collect();
        let catalog = |db: &Connection| {
            db.prepare("SELECT type,name,tbl_name,rootpage,sql FROM sqlite_master WHERE name NOT IN ('receipt_deadline_backfill','invitations_confirmation_scan','invitations_confirmation_pending_seat','invitations_confirmation_pending_thread','warning_conditions_invitation_origin','warning_conditions_invitation_origin_immutable','receipt_discovery_backfill','receipt_due_sparse','receipt_due_sparse_delete','receipt_due_sparse_insert','receipt_due_sparse_update','receipt_warning_origin_attention','receipt_warning_origin_clear','receipt_warning_origin_condition','receipt_warning_origin_receipt_state_insert','receipt_warning_origin_receipt_state_update','receipt_warning_origin_receipts_insert','receipt_warning_origin_receipts_update','receipt_warning_origins','receipt_warning_origins_exact','receipt_warning_origins_seat','receipt_warning_origins_warning','receipts_required_due_ordinal','sqlite_autoindex_receipt_due_sparse_1','sqlite_autoindex_receipt_warning_origins_1') ORDER BY type,name").unwrap()
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<String>>(4)?)))
                .unwrap().map(Result::unwrap).collect::<Vec<_>>()
        };
        let full_catalog = |db: &Connection| {
            db.prepare(
                "SELECT type,name,tbl_name,rootpage,sql FROM sqlite_master ORDER BY type,name",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
        };
        // V44 rebuilds exactly these four body-bearing tables and their implicit indexes.
        // Compare their exact transformed DDL while retaining rootpages for every other object.
        let normalize_capacity = |rows: &mut Vec<(String, String, String, i64, Option<String>)>| {
            for row in rows {
                if [
                    "automatic_handoff_companions",
                    "automatic_bootstrap_parents",
                    "automatic_bootstrap_attachments",
                    "automatic_bootstrap_recoveries",
                ]
                .contains(&row.2.as_str())
                {
                    row.3 = 0;
                    if let Some(sql) = &mut row.4 {
                        *sql = sql
                            .replace(
                                "length(original_json)<=131072",
                                "length(original_json)<=524288",
                            )
                            .replace(
                                "length(attachment_json)<=131072",
                                "length(attachment_json)<=524288",
                            )
                            .replace(
                                "length(result_json)<=262144",
                                "length(result_json)<=2097152",
                            );
                    }
                }
            }
        };
        let old_catalog = catalog(&db);
        // Derive the exact additive warning table declaration from its actual
        // old DDL. Every old constraint/column and rootpage remains compared.
        let mut expected_catalog = old_catalog.clone();
        normalize_capacity(&mut expected_catalog);
        let warning = expected_catalog
            .iter_mut()
            .find(|row| row.1 == "warning_conditions")
            .unwrap();
        let expected = Connection::open_in_memory().unwrap();
        expected
            .execute_batch(warning.4.as_deref().unwrap())
            .unwrap();
        let additions = include_str!("../../migrations/0042_invitation_warning_origin.sql")
            .split("WITH origins AS")
            .next()
            .unwrap();
        expected.execute_batch(additions).unwrap();
        warning.4 = Some(
            expected
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name='warning_conditions'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .unwrap(),
        );
        // Derive the declared45/46 objects independently of initialize's output.
        // The scratch schema needs only historical table declarations and the
        // separately excluded38 origin table; no current catalog is an oracle.
        for row in &expected_catalog {
            if row.0 == "table"
                && !["warning_conditions", "sqlite_sequence"].contains(&row.1.as_str())
            {
                expected.execute_batch(row.4.as_deref().unwrap()).unwrap();
            }
        }
        expected
            .execute_batch(
                include_str!("../../migrations/0043_receipt_warning_origins.sql")
                    .split("CREATE INDEX receipt_warning_origins_seat")
                    .next()
                    .unwrap(),
            )
            .unwrap();
        let before_presentation: std::collections::HashSet<_> =
            catalog(&expected).into_iter().map(|row| row.1).collect();
        expected
            .execute_batch(include_str!(
                "../../migrations/0045_invitation_presentation.sql"
            ))
            .unwrap();
        expected
            .execute_batch(include_str!(
                "../../migrations/0046_presentation_relevance.sql"
            ))
            .unwrap();
        let mut presentation_objects: Vec<_> = catalog(&expected)
            .into_iter()
            .filter(|row| !before_presentation.contains(&row.1))
            .collect();
        assert_eq!(presentation_objects.len(), 118);
        for (kind, count) in [("table", 6), ("index", 16), ("trigger", 96)] {
            assert_eq!(
                presentation_objects
                    .iter()
                    .filter(|row| row.0 == kind)
                    .count(),
                count,
                "declared45/46 {kind}"
            );
        }
        let new_names: std::collections::HashSet<_> = presentation_objects
            .iter()
            .map(|row| row.1.clone())
            .collect();
        // New page allocation is unconstrained; old pages retain their exact
        // witness, apart from the existing four-table44 rebuild exception.
        let normalize_new_pages =
            |rows: &mut Vec<(String, String, String, i64, Option<String>)>| {
                for row in rows {
                    if new_names.contains(&row.1) {
                        row.3 = 0;
                    }
                }
            };
        normalize_new_pages(&mut presentation_objects);
        expected_catalog
            .iter_mut()
            .find(|row| row.1 == "invitations")
            .unwrap()
            .4 = Some(
            expected
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name='invitations'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .unwrap(),
        );
        expected_catalog.extend(presentation_objects);
        expected_catalog.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        let mut expected_rows = before.clone();
        for (table, rows) in tables.iter().zip(&mut expected_rows) {
            if *table == "invitations" {
                for row in rows {
                    row.push(rusqlite::types::Value::Integer(0));
                }
            }
        }
        // CREATE main table succeeds, but INSERT resolves to the obstructing
        // temp view. Failure must roll back the newly created main table too.
        db.execute_batch("CREATE TEMP VIEW receipt_deadline_backfill AS SELECT 1 AS singleton")
            .unwrap();
        assert!(schema::initialize(&db, || UtcMillis(0)).is_err());
        assert!(db.is_autocommit());
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            39
        );
        assert_eq!(
            db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                .unwrap(),
            foreign_keys
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='receipt_deadline_backfill'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(catalog(&db), old_catalog);
        for (table, retained) in tables.iter().zip(&before) {
            assert_eq!(
                &receipt_upgrade_snapshot(&db, table),
                retained,
                "rollback {table}"
            );
        }
        drop(db); // Physical reopen clears only this connection's temp obstruction.
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", foreign_keys)
            .unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            39
        );
        schema::initialize(&db, || UtcMillis(1)).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            schema::LATEST_VERSION
        );
        let initialized_catalog = catalog(&db);
        let initialized_full_catalog = full_catalog(&db);
        assert_eq!(
            {
                let mut rows = initialized_catalog.clone();
                normalize_capacity(&mut rows);
                normalize_new_pages(&mut rows);
                rows
            },
            expected_catalog,
            "composed32..41 preserves historical definitions/pages with exact declared additions and39 rebuilds"
        );
        for (table, retained) in tables.iter().zip(&expected_rows) {
            assert_eq!(&receipt_upgrade_snapshot(&db, table), retained, "{table}");
        }
        assert_eq!(db.query_row("SELECT historical_decision_seq FROM receipt_send_time_cutovers WHERE instance_id='i'", [], |r| r.get::<_, i64>(0)).unwrap(), 12);
        let marker = receipt_upgrade_snapshot(&db, "receipt_deadline_backfill");
        assert_eq!(db.query_row("SELECT physical_after,physical_high_water,prepared_after,prepared_high_water,completed FROM receipt_deadline_backfill WHERE singleton=1", [], |r| Ok((r.get::<_, i64>(0)?,r.get::<_, i64>(1)?,r.get::<_, i64>(2)?,r.get::<_, i64>(3)?,r.get::<_, i64>(4)?))).unwrap(), (0,1,0,2,0));
        db.execute_batch("UPDATE host_instances SET decision_seq=1000 WHERE id='i'")
            .unwrap();
        drop(db);
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", foreign_keys)
            .unwrap();
        schema::initialize(&db, || UtcMillis(2)).unwrap();
        assert_eq!(
            receipt_upgrade_snapshot(&db, "receipt_deadline_backfill"),
            marker
        );
        assert_eq!(
            receipt_upgrade_snapshot(&db, "receipt_send_time_cutovers"),
            before[7]
        );
        for (table, retained) in tables.iter().zip(&expected_rows) {
            assert_eq!(
                &receipt_upgrade_snapshot(&db, table),
                retained,
                "reopened {table}"
            );
        }
        assert_eq!(
            full_catalog(&db),
            initialized_full_catalog,
            "reopen preserves all objects and newly allocated pages too"
        );
        assert_eq!(
            {
                let mut rows = catalog(&db);
                normalize_capacity(&mut rows);
                normalize_new_pages(&mut rows);
                rows
            },
            expected_catalog
        );
        assert_eq!(
            db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                .unwrap(),
            foreign_keys
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}

fn assert_upgrade_warning_link(db: &Connection, table: &str, message: &str) {
    let warning: Option<String> = db
        .query_row(
            &format!("SELECT warning_message_id FROM {table} WHERE message_id=?1 AND seat_id='s'"),
            [message],
            |r| r.get(0),
        )
        .unwrap();
    let warning = warning.unwrap_or_else(|| panic!("unwarned {table} obligation {message}"));
    assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN warning_conditions c ON c.open_warning_id=m.id WHERE m.id=?1 AND m.kind='warn' AND m.thread_id='t' AND c.condition_kind='receipt' AND c.affected_seat_id='s' AND c.clear_warning_id IS NULL)",[warning],|r|r.get::<_,bool>(0)).unwrap(),"{table} {message} must link the durable open receipt warning");
}

#[test]
fn dormant_upgrade_physical_projection_uses_original_send_time() {
    let (path, db) = dormant_upgrade_fixture(1, 0);
    schema::initialize(&db, || UtcMillis(5000)).unwrap();
    assert_eq!(
        effective_receipt(&db, "old0000", "s")
            .unwrap()
            .unwrap()
            .deadline_at,
        Some(1300)
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dormant_upgrade_discovers_physical_and_unmaterialized_manifest_without_availability() {
    let (path, mut db) = dormant_upgrade_fixture(1, 1);
    let mut retained = receipt_upgrade_snapshot(&db, "messages");
    for row in &mut retained {
        row.push(rusqlite::types::Value::Null);
    }
    schema::initialize(&db, || UtcMillis(5000)).unwrap();
    let context = crate::store::connection::StoreContext::new(
        path.clone(),
        std::sync::Arc::new(UpgradeClock),
    );
    let mut cursor = crate::store::receipts::ReceiptDueCursor::default();
    let mut warnings = 0;
    for _ in 0..5 {
        let page = crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
        assert!(page.inspected <= 100);
        warnings += page.warnings;
        if !page.more {
            break;
        }
    }
    assert_eq!(
        warnings, 1,
        "one recipient/thread warning coalesces both obligations"
    );
    assert_eq!(db.query_row("SELECT (SELECT count(*) FROM receipts WHERE warning_message_id IS NOT NULL)+(SELECT count(*) FROM receipt_state WHERE warning_message_id IS NOT NULL)",[],|r|r.get::<_,i64>(0)).unwrap(),2,"both dormant source classes must reach indexed discovery");
    assert_upgrade_warning_link(&db, "receipts", "old0000");
    assert_upgrade_warning_link(&db, "receipt_state", "old0001");
    assert_eq!(
        receipt_upgrade_snapshot(&db, "messages")
            .into_iter()
            .filter(|r| retained.contains(r))
            .count(),
        2
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dormant_upgrade_bounded_batches_survive_restart_and_continuing_arrivals() {
    let (path, mut db) = dormant_upgrade_fixture(250, 250);
    let retained_preparations = receipt_upgrade_snapshot(&db, "send_preparations");
    let retained_recipients = receipt_upgrade_snapshot(&db, "prepared_recipients");
    let retained_manifests = receipt_upgrade_snapshot(&db, "send_manifests");
    schema::initialize(&db, || UtcMillis(5000)).unwrap();
    let context = crate::store::connection::StoreContext::new(
        path.clone(),
        std::sync::Arc::new(UpgradeClock),
    );
    let repair_position = |db: &Connection| -> u64 {
        db.query_row("SELECT a.physical_after+a.prepared_after+b.physical_after+b.sparse_after FROM receipt_deadline_backfill a CROSS JOIN receipt_discovery_backfill b",[],|r|r.get::<_,i64>(0)).unwrap().try_into().unwrap()
    };
    let mut previous_repair = repair_position(&db);
    assert_eq!(previous_repair, 0);
    let mut actual_d = 0u64;
    let mut cursor = crate::store::receipts::ReceiptDueCursor::default();
    let first = crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
    let first_repair = repair_position(&db);
    assert!(first_repair - previous_repair <= 50);
    actual_d += u64::from(first.inspected) - (first_repair - previous_repair);
    previous_repair = first_repair;
    assert_eq!(first.inspected, 100);
    assert!(
        first.more,
        "historical backlog must request immediate continuation"
    );
    let indexed: i64 = db.query_row("SELECT (SELECT count(*) FROM receipts WHERE deadline_at IS NOT NULL)+(SELECT count(*) FROM receipt_state WHERE deadline_at IS NOT NULL)",[],|r|r.get(0)).unwrap();
    assert!(
        indexed > 0 && indexed <= 100,
        "backfill must commit one bounded prefix: {indexed}"
    );
    drop(db);
    db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    schema::initialize(&db, || UtcMillis(9000)).unwrap();
    cursor = crate::store::receipts::ReceiptDueCursor::default();
    let mut finished = false;
    let initial_warned: i64 = db
        .query_row(
            "SELECT count(*) FROM receipts WHERE warning_message_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut captured_arrival_warned_before_repair = false;
    let mut turns = 0;
    for turn in 0..20 {
        // New due work keeps arriving above the captured historical high-water.
        let sequence: i64 = db
            .query_row("SELECT next_sequence FROM threads WHERE id='t'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let decision: i64 = db
            .query_row(
                "SELECT decision_seq+1 FROM host_instances WHERE id='i'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let message = format!("arrival{turn}");
        db.execute(
            "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
            [decision],
        )
        .unwrap();
        db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i',?1,'t',?2,'ordinary','new body',900,?3)",rusqlite::params![message,sequence,decision]).unwrap();
        db.execute("UPDATE threads SET next_sequence=next_sequence+1", [])
            .unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,deadline_at) VALUES(?1,'t','s','pending',300,1200)",[&message]).unwrap();
        let page = crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
        assert!(page.inspected <= 100);
        let next_repair = repair_position(&db);
        let repair = next_repair - previous_repair;
        assert!(
            repair <= 50,
            "all four producers share the repair half allowance"
        );
        actual_d += u64::from(page.inspected) - repair;
        previous_repair = next_repair;
        let warned: i64 = db.query_row("SELECT (SELECT count(*) FROM receipts WHERE message_id GLOB 'old*' AND warning_message_id IS NOT NULL)+(SELECT count(*) FROM receipt_state WHERE message_id GLOB 'old*' AND warning_message_id IS NOT NULL)",[],|r|r.get(0)).unwrap();
        if turn == 0 {
            assert!(
                warned < 500,
                "fixture must still have historical repair backlog"
            );
            assert!(
                warned > initial_warned,
                "ordinary due work progresses while historical repair remains unfinished"
            );
            assert_eq!(
                page.inspected, 100,
                "the full shared repair/due allowance remains available"
            );
        }
        turns += 1;
        let combined: bool = db
            .query_row(
                "SELECT completed FROM receipt_discovery_backfill",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let captured_arrival: bool = db
            .query_row(
                "SELECT warning_message_id IS NOT NULL FROM receipts WHERE message_id='arrival0'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if !combined && captured_arrival {
            captured_arrival_warned_before_repair = true;
        }
        if warned == 500 {
            finished = true;
            break;
        }
        assert!(
            page.more,
            "initial overdue obligations disappeared after turn {turn}: {warned}"
        );
    }
    assert!(
        finished,
        "finite historical backlog starved by arrivals or restart"
    );
    assert!(
        captured_arrival_warned_before_repair,
        "a finite arrival inside the restarted ordinal snapshot is discovered before combined repair exhaustion"
    );
    // H=1001: Task5 physical250/prepared251 plus35 physical250/sparse250.
    // D counts actual ordinary candidate visits, including repeats across the
    // restart and the single combined reset; it is never a distinct-row count.
    assert!(previous_repair <= 1001);
    let receipt_turn_bound = 1001u64.div_ceil(50) + 2 + actual_d.div_ceil(100) + 1;
    assert!(
        turns < receipt_turn_bound,
        "actual receipt turns respect the upgrade envelope: H=1001, visitedH={previous_repair}, D={actual_d}, turns={}",
        turns + 1
    );
    for index in 0..500 {
        assert_upgrade_warning_link(
            &db,
            if index < 250 {
                "receipts"
            } else {
                "receipt_state"
            },
            &format!("old{index:04}"),
        );
    }
    assert_eq!(db.query_row("SELECT physical_after,physical_high_water,prepared_after,prepared_high_water,completed FROM receipt_deadline_backfill",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,bool>(4)?))).unwrap(),(250,250,251,251,true),"new arrivals must not enter the captured repair snapshot");
    assert_eq!(
        receipt_upgrade_snapshot(&db, "send_preparations"),
        retained_preparations
    );
    assert_eq!(
        receipt_upgrade_snapshot(&db, "prepared_recipients"),
        retained_recipients
    );
    assert_eq!(
        receipt_upgrade_snapshot(&db, "send_manifests"),
        retained_manifests
    );
    assert_eq!(
        db.query_row(
            "SELECT historical_decision_seq FROM receipt_send_time_cutovers WHERE instance_id='i'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        510
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM receipt_state WHERE message_id NOT GLOB 'old*'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "unpublished preparation must not materialize"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dormant_upgrade_preserves_running_terminal_waived_and_retained_results() {
    let (path, mut db) = dormant_upgrade_fixture(5, 5);
    let observation = serde_json::json!({
        "harness": "codex", "session": "ns",
        "execution": "00000000-0000-4000-8000-0000000000aa",
        "host_boot": "b", "target_generation": 1, "binding_generation": 1,
        "observed_at": 1100, "decided_at": 1100,
        "provenance": "cooperative_top_level"
    })
    .to_string();
    for index in 1..10 {
        if index == 5 {
            continue;
        }
        let message = format!("old{index:04}");
        let (state, available, required) = match index % 5 {
            1 => ("pending", Some(9700), 1),
            2 => ("acked", None, 1),
            3 => ("recipient_retired", None, 1),
            4 => ("pending", None, 0),
            _ => unreachable!(),
        };
        let actor = (state == "acked").then_some("s");
        let generation = actor.map(|_| 1);
        let proof = actor.map(|_| observation.as_str());
        let acked = actor.map(|_| 1100);
        if index < 5 {
            db.execute("UPDATE receipts SET state=?1,available_at=?2,deadline_at=?3,ack_required=?4,ack_actor_seat_id=?5,ack_generation=?6,ack_observation=?7,acked_at=?8 WHERE message_id=?9",rusqlite::params![state,available,available.map(|at|at+300),required,actor,generation,proof,acked,message]).unwrap();
        } else {
            db.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at,ack_required,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES(?1,'s',?2,?3,?4,?5,?6,?7,?8,?9)",rusqlite::params![message,state,available,available.map(|at|at+300),required,actor,generation,proof,acked]).unwrap();
        }
    }
    let result = "{\"kind\":\"acknowledged\",\"data\":{\"acknowledged\":[\"old0002\"],\"already_acknowledged\":[]}}";
    db.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES('actor','retained',zeroblob(32),?1,1100)",[result]).unwrap();
    let unchanged = |db: &Connection, table: &str| {
        receipt_upgrade_snapshot(db,table).into_iter().filter(|row| {
        let message_column = if table == "receipts" { 2 } else { 1 };
        !matches!(&row[message_column], rusqlite::types::Value::Text(id) if id == "old0000" || id == "old0005")
    }).collect::<Vec<_>>()
    };
    let mut before_physical = unchanged(&db, "receipts");
    for row in &mut before_physical {
        row.push(rusqlite::types::Value::Null);
    }
    let mut before_sparse = unchanged(&db, "receipt_state");
    for row in &mut before_sparse {
        row.push(rusqlite::types::Value::Null);
    }
    let retained = receipt_upgrade_snapshot(&db, "operations");
    schema::initialize(&db, || UtcMillis(5000)).unwrap();
    let context = crate::store::connection::StoreContext::new(
        path.clone(),
        std::sync::Arc::new(UpgradeClock),
    );
    let mut cursor = crate::store::receipts::ReceiptDueCursor::default();
    let mut warnings = 0;
    for _ in 0..10 {
        let page = crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
        warnings += page.warnings;
        if !page.more {
            break;
        }
    }
    assert_eq!(
        warnings, 1,
        "one recipient/thread warning coalesces dormant obligations"
    );
    assert_eq!(db.query_row("SELECT (SELECT count(*) FROM receipts WHERE warning_message_id IS NOT NULL)+(SELECT count(*) FROM receipt_state WHERE warning_message_id IS NOT NULL)",[],|r|r.get::<_,i64>(0)).unwrap(),2,"only dormant required obligations expire");
    assert_upgrade_warning_link(&db, "receipts", "old0000");
    assert_upgrade_warning_link(&db, "receipt_state", "old0005");
    assert_eq!(unchanged(&db, "receipts"), before_physical);
    assert_eq!(unchanged(&db, "receipt_state"), before_sparse);
    assert_eq!(receipt_upgrade_snapshot(&db, "operations"), retained);
    schema::initialize(&db, || UtcMillis(9000)).unwrap();
    for message in ["old0002", "old0007"] {
        let receipt = effective_receipt(&db, message, "s").unwrap().unwrap();
        assert_eq!(receipt.deadline_at, None);
        assert_eq!(
            receipt.ack_observation.as_deref(),
            Some(observation.as_str())
        );
    }
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dormant_upgrade_all_terminal_backlog_counts_staged_rows_and_completes_once() {
    let (path, mut db) = dormant_upgrade_fixture(250, 250);
    db.execute("UPDATE receipts SET state='acked'", []).unwrap();
    db.execute("INSERT INTO receipt_state(message_id,seat_id,state) SELECT sm.message_id,pr.seat_id,'acked' FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id",[]).unwrap();
    let mut before_physical = receipt_upgrade_snapshot(&db, "receipts");
    for row in &mut before_physical {
        row.push(rusqlite::types::Value::Null);
    }
    let mut before_sparse = receipt_upgrade_snapshot(&db, "receipt_state");
    for row in &mut before_sparse {
        row.push(rusqlite::types::Value::Null);
    }
    schema::initialize(&db, || UtcMillis(5000)).unwrap();
    let mut visited = 0;
    let mut transitions = 0;
    let mut done = false;
    for _ in 0..8 {
        let page = crate::store::materialization::backfill_receipt_deadlines(&mut db, 100).unwrap();
        assert!(page.inspected <= 100);
        visited += page.inspected;
        transitions += u16::from(page.completed);
        if !page.more {
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(
        visited, 501,
        "every physical/prepared row counts, including unpublished p"
    );
    assert_eq!(transitions, 1);
    let again = crate::store::materialization::backfill_receipt_deadlines(&mut db, 100).unwrap();
    assert_eq!(
        (again.inspected, again.more, again.completed),
        (0, false, false)
    );
    assert_eq!(receipt_upgrade_snapshot(&db, "receipts"), before_physical);
    assert_eq!(
        receipt_upgrade_snapshot(&db, "receipt_state"),
        before_sparse
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dormant_upgrade_completion_and_due_warning_rollback_together_then_restart() {
    let (path, mut db) = dormant_upgrade_fixture(1, 1);
    schema::initialize(&db, || UtcMillis(5000)).unwrap();
    let before = receipt_upgrade_snapshot(&db, "receipt_deadline_backfill");
    let receipts = receipt_upgrade_snapshot(&db, "receipts");
    db.execute_batch("CREATE TRIGGER fail_upgrade_due BEFORE INSERT ON warning_conditions BEGIN SELECT RAISE(ABORT,'test due failure'); END;").unwrap();
    let context = crate::store::connection::StoreContext::new(
        path.clone(),
        std::sync::Arc::new(UpgradeClock),
    );
    let mut cursor = crate::store::receipts::ReceiptDueCursor {
        high_water_ordinal: 1,
        after_deadline: Some(9000),
        after_ordinal: 1,
        ..Default::default()
    };
    let saved_cursor = cursor.clone();
    let error = crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap_err();
    assert!(error.detail.contains("test due failure"), "{error:?}");
    assert_eq!(cursor, saved_cursor);
    assert_eq!(
        receipt_upgrade_snapshot(&db, "receipt_deadline_backfill"),
        before
    );
    assert_eq!(receipt_upgrade_snapshot(&db, "receipts"), receipts);
    assert_eq!(
        db.query_row("SELECT count(*) FROM receipt_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_upgrade_due").unwrap();
    drop(db);
    db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    schema::initialize(&db, || UtcMillis(9000)).unwrap();
    let page = crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
    assert_eq!(page.warnings, 1);
    assert_eq!(db.query_row("SELECT (SELECT count(*) FROM receipts WHERE warning_message_id IS NOT NULL)+(SELECT count(*) FROM receipt_state WHERE warning_message_id IS NOT NULL)",[],|r|r.get::<_,i64>(0)).unwrap(),2,"completion must reset the old due key before same-call selection");
    assert_upgrade_warning_link(&db, "receipts", "old0000");
    assert_upgrade_warning_link(&db, "receipt_state", "old0001");
    let again = crate::store::materialization::backfill_receipt_deadlines(&mut db, 100).unwrap();
    assert_eq!(
        (again.inspected, again.more, again.completed),
        (0, false, false)
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dormant_upgrade_keeps_due_cursor_until_the_one_completion_transition() {
    let (path, mut db) = dormant_upgrade_fixture(250, 250);
    schema::initialize(&db, || UtcMillis(5000)).unwrap();
    let context = crate::store::connection::StoreContext::new(
        path.clone(),
        std::sync::Arc::new(UpgradeClock),
    );
    let mut cursor = crate::store::receipts::ReceiptDueCursor {
        high_water_ordinal: 250,
        after_deadline: Some(9000),
        after_ordinal: 250,
        ..Default::default()
    };
    let first = crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
    assert_eq!((first.inspected, first.warnings, first.more), (50, 0, true));
    assert_eq!(
        cursor.after_deadline,
        Some(9000),
        "repair must not restart traversal on every batch"
    );
    let mut warnings = 0;
    let mut done = false;
    for _ in 0..25 {
        let page = crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
        assert!(page.inspected <= 100);
        warnings += page.warnings;
        if !page.more {
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(
        warnings, 1,
        "one recipient/thread warning coalesces all obligations"
    );
    assert_eq!(db.query_row("SELECT (SELECT count(*) FROM receipts WHERE warning_message_id IS NOT NULL)+(SELECT count(*) FROM receipt_state WHERE warning_message_id IS NOT NULL)",[],|r|r.get::<_,i64>(0)).unwrap(),500,"fresh completion walk must catch all repaired deadlines behind old key");
    for index in 0..500 {
        assert_upgrade_warning_link(
            &db,
            if index < 250 {
                "receipts"
            } else {
                "receipt_state"
            },
            &format!("old{index:04}"),
        );
    }
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dormant_upgrade_removed_tails_finalize_without_spending_row_allowance() {
    let (path, mut db) = dormant_upgrade_fixture(2, 2);
    schema::initialize(&db, || UtcMillis(5000)).unwrap();
    db.execute("DELETE FROM receipts WHERE ordinal=2", [])
        .unwrap();
    db.execute(
        "DELETE FROM prepared_recipients WHERE preparation_id='oldp0003'",
        [],
    )
    .unwrap();
    let first = crate::store::materialization::backfill_receipt_deadlines(&mut db, 1).unwrap();
    assert_eq!(
        (first.inspected, first.more, first.completed),
        (1, true, false)
    );
    let second = crate::store::materialization::backfill_receipt_deadlines(&mut db, 1).unwrap();
    assert_eq!(
        (second.inspected, second.more, second.completed),
        (1, true, false)
    ); // unpublished p
    let third = crate::store::materialization::backfill_receipt_deadlines(&mut db, 1).unwrap();
    assert_eq!(
        (third.inspected, third.more, third.completed),
        (1, true, false)
    );
    let final_page = crate::store::materialization::backfill_receipt_deadlines(&mut db, 1).unwrap();
    assert_eq!(
        (final_page.inspected, final_page.more, final_page.completed),
        (0, false, true)
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dormant_upgrade_empty_snapshot_keeps_full_ordinary_due_allowance() {
    let (path, mut db) = dormant_upgrade_fixture(0, 0);
    db.execute("DELETE FROM prepared_recipients", []).unwrap();
    schema::initialize(&db, || UtcMillis(5000)).unwrap();
    for sequence in 1..=100 {
        let message = format!("new{sequence}");
        db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i',?1,'t',?2,'ordinary','new body',1000,?2)",rusqlite::params![message,sequence]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,deadline_at) VALUES(?1,'t','s','pending',300,1300)",[message]).unwrap();
    }
    db.execute_batch(
        "UPDATE threads SET next_sequence=101; UPDATE host_instances SET decision_seq=100;",
    )
    .unwrap();
    let context = crate::store::connection::StoreContext::new(
        path.clone(),
        std::sync::Arc::new(UpgradeClock),
    );
    let mut cursor = crate::store::receipts::ReceiptDueCursor::default();
    let page = crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
    assert_eq!((page.inspected, page.warnings), (100, 1));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM receipts WHERE warning_message_id IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        100,
        "full allowance must mark every new due obligation"
    );
    for sequence in 1..=100 {
        assert_upgrade_warning_link(&db, "receipts", &format!("new{sequence}"));
    }
    let completed =
        crate::store::materialization::backfill_receipt_deadlines(&mut db, 100).unwrap();
    assert_eq!(
        (completed.inspected, completed.more, completed.completed),
        (0, false, false)
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dormant_upgrade_ack_before_repair_preserves_original_not_extended_deadline() {
    use crate::protocol::authority::{
        CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef,
    };
    use crate::protocol::ids::{
        ExecutionId, HostTargetId, MessageId, NativeSessionId, OperationId, SeatId,
    };
    for physical in [true, false] {
        let (path, mut db) = dormant_upgrade_fixture(usize::from(physical), usize::from(!physical));
        // Actual pre-upgrade NULL/NULL storage, including an existing sparse
        // pending row; ACK is possible before this binding first registers.
        if !physical {
            db.execute("INSERT INTO receipt_state(message_id,seat_id,state) VALUES('old0000','s','pending')",[]).unwrap();
        }
        db.execute_batch("UPDATE host_instances SET host_boot='b',host_epoch=1 WHERE id='i'; UPDATE seats SET target_id='p',target_generation=1 WHERE id='s'; INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i','p','b',1,1,1100,'fresh','term-p','inc','coherent_enumeration',1); INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES('s',1,'p','b',1,1,'codex','ns','00000000-0000-4000-8000-0000000000aa','cooperative_top_level',1100,'term-p','inc'); INSERT INTO catch_up(seat_id,thread_id,frontier_seq,binding_generation,execution_id,entered_at,extension_until,state) VALUES('s','t',0,1,'00000000-0000-4000-8000-0000000000aa',1100,6000,'active');").unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        schema::initialize(&db, || UtcMillis(5000)).unwrap();
        let before = effective_receipt(&db, "old0000", "s").unwrap().unwrap();
        assert_eq!(before.deadline_at, Some(1300));
        assert_eq!(
            crate::store::receipts::effective_deadline(&db, &before).unwrap(),
            Some(6000)
        );
        let request = crate::protocol::commands::Ack {
            messages: vec![MessageId::new("old0000")],
            operation: OperationId::new("upgrade-ack"),
            claim: CallerClaim {
                instance: "i".into(),
                seat: SeatId::new("s"),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new("ns"),
                execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
                target: HostTargetId::new("p"),
                agent_id: None,
                liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
            },
        };
        let budget = CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        };
        let digest =
            schema::canonical_digest(&crate::store::receipts::ack_payload(&request)).unwrap();
        let mut permit = MutationPermit::cooperative(
            request.claim.clone(),
            request.operation.clone(),
            ObligationRef::CheckIn(request.claim.seat.clone()),
            digest,
            crate::protocol::time::MonoInstant(0),
            (1, 0),
            budget.clone(),
        );
        let context = crate::store::connection::StoreContext::new(
            path.clone(),
            std::sync::Arc::new(UpgradeClock),
        );
        crate::store::receipts::ack(&context, &mut db, &budget, &request, &mut permit).unwrap();
        let after = effective_receipt(&db, "old0000", "s").unwrap().unwrap();
        assert_eq!(after.state, EffectiveReceiptState::Acknowledged);
        assert_eq!(
            after.deadline_at,
            Some(1300),
            "ACK before repair must retain the original clock; physical={physical}"
        );
        assert_eq!(after.ack_actor_seat_id.as_deref(), Some("s"));
        assert_eq!(after.ack_generation, Some(1));
        assert_eq!(after.acked_at, Some(5000));
        assert_eq!(
            db.query_row(
                "SELECT extension_until FROM catch_up WHERE seat_id='s' AND thread_id='t'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            6000
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "extension still suppresses overdue warning"
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once() {
    for version in [28, 37] {
        let db = v28_fixture();
        if version == 37 {
            db.execute_batch(include_str!("../../migrations/0029_direct_threads.sql"))
                .unwrap();
            db.execute_batch(include_str!("../../migrations/0030_automatic_handoff.sql"))
                .unwrap();
            db.execute_batch(include_str!("../../migrations/0031_seat_wake_on.sql"))
                .unwrap();
            apply_published_bindings32_to35(&db);
            db.execute_batch(include_str!("../../migrations/0036_in_turn_offer.sql"))
                .unwrap();
            db.execute_batch(include_str!(
                "../../migrations/0037_invitation_confirmation.sql"
            ))
            .unwrap();
            db.pragma_update(None, "user_version", 37).unwrap();
            db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES ('modern','t','s',1,'pending',1,100,50,150,'require_ack','s'),('quiet','t','s',2,'pending',2,100,50,150,'lazy','s');").unwrap();
        }
        for (index, (name, state, available, required)) in [
            ("running", "pending", Some(1200), 1),
            ("dormant", "pending", None, 1),
            ("acked", "acked", Some(1200), 1),
            ("acked_absent", "acked", None, 1),
            ("retired", "recipient_retired", None, 1),
            ("waived", "pending", None, 0),
        ]
        .into_iter()
        .enumerate()
        {
            db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i',?1,'t',?2,'ordinary','retained body',1000,?2)",rusqlite::params![name,index as i64+1]).unwrap();
            db.execute("INSERT INTO receipts(ordinal,message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,ack_required,ack_observation) VALUES(?1,?2,'t','s',?3,300,?4,?5,?6,'retained provenance')",rusqlite::params![index as i64+20,name,state,available,available.map(|at|at+300),required]).unwrap();
            db.execute("INSERT INTO receipt_state(rowid,message_id,seat_id,state,available_at,deadline_at,ack_required,ack_observation) VALUES(?1,?2,'s',?3,?4,?5,?6,'retained provenance')",rusqlite::params![index as i64+40,name,state,available,available.map(|at|at+300),required]).unwrap();
        }
        db.execute_batch("UPDATE sqlite_sequence SET seq=99 WHERE name='receipts'; CREATE TABLE receipt_upgrade_child(ordinal INTEGER REFERENCES receipts(ordinal), message_id TEXT, seat_id TEXT, FOREIGN KEY(message_id,seat_id) REFERENCES receipt_state(message_id,seat_id)); INSERT INTO receipt_upgrade_child VALUES(20,'running','s');").unwrap();
        let tables = [
            "receipts",
            "receipt_state",
            "messages",
            "send_preparations",
            "prepared_recipients",
            "receipt_upgrade_child",
            "digest_pending_manifest_receipts",
        ];
        let before: Vec<_> = tables
            .iter()
            .map(|name| receipt_upgrade_snapshot(&db, name))
            .collect();
        let objects = |db: &Connection| {
            // These eight migration42 additions are separate from the exact
            // historical catalog; all other receipt objects remain compared.
            let mut statement=db.prepare("SELECT name,sql FROM sqlite_master WHERE tbl_name IN ('receipts','receipt_state') AND type IN ('trigger','index') AND name NOT IN ('receipt_due_sparse_delete','receipt_due_sparse_insert','receipt_due_sparse_update','receipt_warning_origin_receipt_state_insert','receipt_warning_origin_receipt_state_update','receipt_warning_origin_receipts_insert','receipt_warning_origin_receipts_update','receipts_required_due_ordinal') AND sql IS NOT NULL ORDER BY name").unwrap();
            statement
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let old_objects = objects(&db);
        let presentation_triggers = [
            (
                "presentation_fence_receipt_state_delete",
                "receipt_state",
                "DELETE",
            ),
            (
                "presentation_fence_receipt_state_insert",
                "receipt_state",
                "INSERT",
            ),
            (
                "presentation_fence_receipt_state_update",
                "receipt_state",
                "UPDATE",
            ),
            ("presentation_fence_receipts_delete", "receipts", "DELETE"),
            ("presentation_fence_receipts_insert", "receipts", "INSERT"),
            ("presentation_fence_receipts_update", "receipts", "UPDATE"),
        ];
        schema::initialize(&db, || UtcMillis(0)).unwrap();
        let (new_triggers, retained_objects): (Vec<_>, Vec<_>) =
            objects(&db).into_iter().partition(|(name, _)| {
                presentation_triggers
                    .iter()
                    .any(|(expected, _, _)| name == expected)
            });
        assert_eq!(
            retained_objects, old_objects,
            "v{version} historical catalog"
        );
        // Assert the six migration46 additions independently, including their
        // event, table and fail-closed fence updates, rather than ignoring them.
        let expected_triggers: Vec<_> = presentation_triggers
            .iter()
            .map(|(name, table, event)| {
                (
                    (*name).to_owned(),
                    format!(
                        r#"CREATE TRIGGER {name} AFTER {event} ON {table}
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END"#
                    ),
                )
            })
            .collect();
        assert_eq!(
            new_triggers, expected_triggers,
            "v{version} migration46 triggers"
        );
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            schema::LATEST_VERSION
        );
        for (name, mut expected) in tables.iter().zip(before) {
            if version == 28 && ["receipts", "receipt_state", "messages"].contains(name) {
                for row in &mut expected {
                    row.push(rusqlite::types::Value::Null);
                }
            }
            assert_eq!(receipt_upgrade_snapshot(&db, name), expected, "{name}");
        }
        assert_eq!(
            db.query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='receipts'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            99
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        assert!(
            db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                .unwrap()
        );
        db.execute_batch("UPDATE host_instances SET decision_seq=99; UPDATE receipt_state SET available_at=NULL,deadline_at=1300 WHERE message_id='dormant'; UPDATE receipts SET available_at=NULL,deadline_at=1300 WHERE message_id='dormant';").unwrap();
        assert!(
            db.execute("UPDATE receipts SET ordinal=200 WHERE ordinal=20", [])
                .is_err(),
            "immutable ordinal trigger lost"
        );
        schema::initialize(&db, || UtcMillis(9999)).unwrap();
        assert_eq!(
        db.query_row(
            "SELECT historical_decision_seq FROM receipt_send_time_cutovers WHERE instance_id='i'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        10
    );
        if version == 37 {
            let invitations: Vec<(String, String, String, i64)> = db.prepare("SELECT id,confirmation,inviter_seat_id,deadline_at FROM invitations ORDER BY id").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().collect::<Result<_, _>>().unwrap();
            assert_eq!(
                invitations,
                [
                    ("modern".into(), "require_ack".into(), "s".into(), 150),
                    ("quiet".into(), "lazy".into(), "s".into(), 150)
                ]
            );
            assert!(
                db.execute(
                    "UPDATE invitations SET confirmation='lazy' WHERE id='modern'",
                    []
                )
                .is_err()
            );
            assert!(
                db.execute(
                    "UPDATE invitations SET inviter_seat_id=NULL WHERE id='modern'",
                    []
                )
                .is_err()
            );
            let due: Vec<String> = db.prepare("SELECT id FROM invitations INDEXED BY invitations_confirmation_due WHERE state='pending' AND warning_message_id IS NULL AND reject_recorded=0 AND (confirmation IS NULL OR confirmation='require_ack') ORDER BY deadline_at,ordinal").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
            assert_eq!(due, ["modern"]);
        }
    }
}

#[test]
fn receipt_send_time_upgrade_failure_rolls_back_constraint_and_cutover_then_retries() {
    for version in [28, 37] {
        for foreign_keys in [false, true] {
            let db = v28_fixture();
            if version == 37 {
                db.execute_batch(include_str!("../../migrations/0029_direct_threads.sql"))
                    .unwrap();
                db.execute_batch(include_str!("../../migrations/0030_automatic_handoff.sql"))
                    .unwrap();
                db.execute_batch(include_str!("../../migrations/0031_seat_wake_on.sql"))
                    .unwrap();
                apply_published_bindings32_to35(&db);
                db.execute_batch(include_str!("../../migrations/0036_in_turn_offer.sql"))
                    .unwrap();
                db.execute_batch(include_str!(
                    "../../migrations/0037_invitation_confirmation.sql"
                ))
                .unwrap();
                db.pragma_update(None, "user_version", 37).unwrap();
            }
            db.execute_batch(
                "CREATE TABLE receipt_upgrade_child(ordinal INTEGER REFERENCES receipts(ordinal));",
            )
            .unwrap();
            db.pragma_update(None, "foreign_keys", false).unwrap();
            db.execute("INSERT INTO receipt_upgrade_child VALUES(999)", [])
                .unwrap();
            db.pragma_update(None, "foreign_keys", foreign_keys)
                .unwrap();
            assert!(schema::initialize(&db, || UtcMillis(0)).is_err());
            assert!(db.is_autocommit());
            assert_eq!(
                db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                    .unwrap(),
                foreign_keys
            );
            // Receipt send time is migration 38: the last committed step is
            // invitation confirmation (37), never a partial receipt rebuild.
            assert_eq!(
                db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                    .unwrap(),
                37
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='receipt_send_time_cutovers'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            for table in ["receipts", "receipt_state"] {
                let ddl: String = db
                    .query_row(
                        "SELECT sql FROM sqlite_master WHERE name=?1",
                        [table],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert!(ddl.contains("CHECK((available_at IS NULL) = (deadline_at IS NULL))"));
            }
            db.execute("DELETE FROM receipt_upgrade_child", []).unwrap();
            schema::initialize(&db, || UtcMillis(0)).unwrap();
            assert_eq!(
                db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                    .unwrap(),
                schema::LATEST_VERSION
            );
            schema::initialize(&db, || UtcMillis(1)).unwrap();
            assert_eq!(
                db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                    .unwrap(),
                foreign_keys
            );
        }
    }
}

#[test]
fn historical_sparse_retired_before_cutover_preserves_absent_deadline() {
    let db = v28_fixture();
    db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,5); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',5,1000,1,0,1,0); UPDATE seats SET state='retired',retired_at=1100,retired_seq=6 WHERE id='s'; INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES('s',11,1200,1,'cooperative_top_level');").unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    let receipt = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(receipt.state, EffectiveReceiptState::RecipientRetired);
    assert_eq!(receipt.deadline_at, None);
    assert_eq!(receipt.available_at, None);
}

#[test]
fn historical_sparse_acknowledged_absent_deadline_survives_upgrade_and_later_availability() {
    let db = v28_fixture();
    // A live cooperative binding can ACK before its first registration. On
    // v28 this leaves both availability and deadline absent at settlement.
    db.execute_batch("UPDATE host_instances SET host_boot='b',host_epoch=1 WHERE id='i'; UPDATE seats SET target_id='p',target_generation=1 WHERE id='s'; INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i','p','b',1,1,1100,'fresh','term-p','inc','coherent_enumeration',1); INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES('s',1,'p','b',1,1,'codex','ns','00000000-0000-4000-8000-0000000000aa','cooperative_top_level',1100,'term-p','inc'); INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','retained body',1000,5); INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',5,1000,1,0,1,0);").unwrap();
    let observation = serde_json::json!({
        "harness": "codex", "session": "ns",
        "execution": "00000000-0000-4000-8000-0000000000aa",
        "host_boot": "b", "target_generation": 1, "binding_generation": 1,
        "observed_at": 1100, "decided_at": 1100,
        "provenance": "cooperative_top_level"
    })
    .to_string();
    db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES('m','s','acked','s',1,?1,1100)",[&observation]).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        28
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    let mut retained = receipt_upgrade_snapshot(&db, "receipt_state");
    for row in &mut retained {
        row.push(rusqlite::types::Value::Null);
    }
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    assert_eq!(
        effective_receipt(&db, "m", "s")
            .unwrap()
            .unwrap()
            .deadline_at,
        None
    );
    // A real later availability event may update observed availability, but it
    // cannot invent a running clock for a historical acknowledged receipt.
    db.execute_batch("UPDATE host_instances SET decision_seq=11 WHERE id='i'; UPDATE occupant_bindings SET registered_at=1200 WHERE seat_id='s' AND generation=1; INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES('s',11,1200,1,'cooperative_top_level');").unwrap();
    schema::initialize(&db, || UtcMillis(1200)).unwrap();
    let receipt = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(receipt.state, EffectiveReceiptState::Acknowledged);
    assert_eq!(receipt.deadline_at, None);
    assert_eq!(receipt.ack_actor_seat_id.as_deref(), Some("s"));
    assert_eq!(receipt.ack_generation, Some(1));
    assert_eq!(
        receipt.ack_observation.as_deref(),
        Some(observation.as_str())
    );
    assert_eq!(receipt.acked_at, Some(1100));
    assert_eq!(receipt_upgrade_snapshot(&db, "receipt_state"), retained);
}

fn apply_published_bindings32_to35(db: &Connection) {
    for (version, migration) in [
        (
            32,
            include_str!("../../migrations/0032_session_bindings.sql"),
        ),
        (
            33,
            include_str!("../../migrations/0033_archival_scan_scope.sql"),
        ),
        (
            34,
            include_str!("../../migrations/0034_binding_delivery_state.sql"),
        ),
        (
            35,
            include_str!("../../migrations/0035_binding_attention_frontiers.sql"),
        ),
    ] {
        db.execute_batch(migration).unwrap();
        db.pragma_update(None, "user_version", version).unwrap();
    }
}

// Preserve the published test key and first observed availability oracle.
// The appended send-time policy freezes deadline1300 independently of that anchor.
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
    assert_eq!(before.deadline_at, Some(1300));
    db.execute_batch("\
        INSERT INTO seat_availability(seat_id, decision_seq, decision_at, binding_generation, observation_provenance) VALUES ('s', 11, 1200, 1, 'verified');\
        INSERT INTO seat_availability(seat_id, decision_seq, decision_at, binding_generation, observation_provenance) VALUES ('s', 12, 500, 1, 'verified');\
    ").unwrap();
    let after = effective_receipt(&db, "m", "s").unwrap().unwrap();
    assert_eq!(after.available_at, Some(1200));
    assert_eq!(after.deadline_at, Some(1300));
    assert_eq!(after.state, EffectiveReceiptState::Pending);
}
