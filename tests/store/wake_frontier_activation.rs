use herdr_threads::{
    ports::{LogicalAttentionFrontier, LogicalPublicationKey, PrepareWakeIntent, StorePort},
    protocol::{
        pagination::PageRequest,
        time::{CallBudget, MonoInstant},
    },
};

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Default::default(),
    }
}

#[test]
fn prepare_inserts_every_canonical_frontier_component() {
    let (store, _path, initial, preparation) = super::wake_intents::fixture();
    let db = store.context().open_writer().unwrap();
    // Keep the fixture's pending invitation on t; ordinary attention belongs to
    // a joined thread. Publication builds the binding's lazy pending view.
    db.execute_batch(
        "DELETE FROM wake_work;
         INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('mail','34750f98bd59fcfc946da45aaabe933b','mail','goal',0,0,0);
         INSERT INTO memberships(thread_id,seat_id,state) VALUES ('mail','s','joined');
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('mail','s',1,1);
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('receipt-message','34750f98bd59fcfc946da45aaabe933b','mail',1,'ordinary','receipt',2,0);
         INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('receipt-message','mail','s','pending',100);
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_seq,decision_at,event_offset) VALUES ('warning','34750f98bd59fcfc946da45aaabe933b','mail',2,'warn','{}',3,0,7);
         INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('warning',3,'mail',100,'s','invitation','inv');
         INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES ('warning','s',1);
         INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES ('binding','mail',0);
         INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES ('prep-lazy','34750f98bd59fcfc946da45aaabe933b','scope','prep-lazy',zeroblob(32),'mail',0,0,0,0,0,0,0,'building','lazy',0);
         INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES ('prep-lazy','msg-lazy','mail','s',0);
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES ('msg-lazy','34750f98bd59fcfc946da45aaabe933b','mail',3,'ordinary',4,'lazy',0,'lazy',0);
         INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('prep-lazy','msg-lazy','34750f98bd59fcfc946da45aaabe933b','mail',4,0,3,0,1,0);
         UPDATE lazy_recipients SET decision_seq=4 WHERE preparation_id='prep-lazy';
         UPDATE local_node SET decision_seq=4;",
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM wake_intents", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "prepare must exercise INSERT, not an existing ledger row"
    );
    drop(db);
    let candidate = StorePort::wake_candidates(
        &store,
        herdr_threads::test_support::test_host_id(),
        PageRequest::default(),
        &budget(),
    )
    .unwrap()
    .items
    .remove(0);
    let reservation = StorePort::reserve_wake(
        &store,
        herdr_threads::test_support::test_host_id(),
        &candidate,
        &budget(),
    )
    .unwrap()
    .unwrap();
    let expected = LogicalAttentionFrontier {
        invitation: initial.attention_witness.frontier().invitation,
        addressed_receipt: Some(LogicalPublicationKey {
            decision_seq: 2,
            event_offset: 0,
        }),
        actionable_warning: Some(LogicalPublicationKey {
            decision_seq: 3,
            event_offset: 7,
        }),
        lazy: Some(4),
    };
    assert!(expected.invitation.is_some());
    assert_eq!(reservation.attention_witness.frontier(), expected);
    let PrepareWakeIntent::Prepared(inserted) =
        StorePort::prepare_wake_intent(&store, &reservation, &preparation, &budget()).unwrap()
    else {
        panic!("fresh canonical reservation must insert an intent");
    };
    assert_eq!(inserted.frontier, expected);
    let persisted = StorePort::wake_intent(&store, &reservation.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(persisted.frontier, expected);
}
