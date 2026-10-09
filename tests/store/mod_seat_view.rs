//! Mod watch store reads and the commit-hook notify (ht-j16.2). Included from
//! `src/store/seats.rs` as `mod_seat_view_tests`.
use super::*;
use crate::{
    ports::{ModBindingView, ModFingerprint, ModStoreReads},
    protocol::{authority::Harness, ids::NativeSessionId, watch::WATCH_BODY_LIMIT_BYTES},
    store::{
        SqliteStore,
        cooperative_checkin_tests::{budget, check_in, claim, fixture, lifecycle},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn seat_s() -> SeatId {
    SeatId::new("s")
}

fn claude_claim(session: &str, execution: &str) -> crate::protocol::authority::CallerClaim {
    let mut claim = claim();
    claim.harness = Harness::Claude;
    claim.native_session = NativeSessionId::new(session);
    claim.execution = crate::protocol::ids::ExecutionId::new(execution);
    claim
}

fn view(store: &SqliteStore) -> crate::ports::ModSeatView {
    store.mod_seat_view(&seat_s(), &budget()).unwrap().unwrap()
}

#[test]
fn view_reports_open_claude_binding_generation_and_native_session() {
    let (store, _conn, _) = fixture();
    // Before any check-in: a resolved seat without a binding.
    let before = view(&store);
    assert!(before.continuity_resolved && !before.retired && !before.held);
    assert_eq!(before.binding, None);
    assert_eq!(before.fingerprint.binding_generation, None);
    // A Claude SessionStart-shaped lifecycle check-in with native session X.
    let result = check_in(
        &store,
        lifecycle(
            claude_claim("session-X", "00000000-0000-4000-8000-000000000001"),
            "op-1",
        ),
    )
    .unwrap();
    assert_eq!(result.context.binding_generation, 1);
    let after = view(&store);
    assert_eq!(
        after.binding,
        Some(ModBindingView {
            generation: 1,
            provenance: "cooperative_top_level".into(),
            harness: "claude".into(),
            native_session: "session-X".into(),
        })
    );
    assert_eq!(after.fingerprint.binding_generation, Some(1));
    assert_eq!(after.state, "resolved");
    // An unknown seat has no view.
    assert_eq!(
        store
            .mod_seat_view(&SeatId::new("nobody"), &budget())
            .unwrap(),
        None
    );
}

#[test]
fn view_reports_held_unresolved_retired() {
    let (store, conn, _) = fixture();
    assert!(!view(&store).held);
    conn.execute(
        "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ('i','p','b',1,'hold')",
        [],
    )
    .unwrap();
    assert!(view(&store).held);
    conn.execute("UPDATE recovery_holds SET released_at=5", [])
        .unwrap();
    assert!(!view(&store).held, "a released hold no longer holds");

    conn.execute("UPDATE seats SET state='unresolved' WHERE id='s'", [])
        .unwrap();
    let unresolved = view(&store);
    assert!(!unresolved.continuity_resolved && !unresolved.retired);
    assert_eq!(unresolved.state, "unresolved");

    conn.execute(
        "UPDATE seats SET state='retired',retired_at=9,retired_seq=1 WHERE id='s'",
        [],
    )
    .unwrap();
    let retired = view(&store);
    assert!(retired.retired && !retired.continuity_resolved);
}

/// Base rows for the call-site cases: seats `s` (the watched seat) and `o`,
/// threads, host decision sequence 100.
fn base(conn: &Connection) {
    conn.execute_batch(
        "UPDATE host_instances SET decision_seq=100;\
         INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('o','i','resolved','native','po',1,1,0);\
         INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t1','i','a','g',0,0),('t2','i','b','g',0,0),('t4','i','d','g',0,0);",
    )
    .unwrap();
}

fn fp(store: &SqliteStore) -> ModFingerprint {
    view(store).fingerprint
}

/// Runs `action` as one writer turn of the store and asserts that the
/// fingerprint of seat `s` changed and that the commit notified the mod
/// observer (the call site's tables are in `MOD_NOTIFY_TABLES`).
fn changes(store: &SqliteStore, _conn: &Connection, name: &str, action: &str) {
    let before = fp(store);
    let notified = counting_observer(store);
    {
        let turn = store.writer(&budget()).unwrap();
        turn.execute_batch(action)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
    let after = fp(store);
    assert_ne!(before, after, "{name}: the fingerprint did not change");
    assert!(
        notified.load(Ordering::SeqCst) >= 1,
        "{name}: the commit did not notify the mod observer"
    );
}

#[test]
fn fingerprint_changes_on_each_call_site() {
    // Ordinary send to the seat, then settlement by an inbox/hook ACK.
    let (store, conn, _) = fixture();
    base(&conn);
    changes(
        &store,
        &conn,
        "ordinary send",
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m1','i','t2',1,'ordinary','b',0,5);\
         INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m1','t2','s','pending',100);",
    );
    let sent = fp(&store);
    assert_eq!(sent.pending_receipts, 1);
    assert_eq!(sent.max_pending_ordinal, 5);
    changes(
        &store,
        &conn,
        "settlement by another path (hook or inbox ACK)",
        "UPDATE receipts SET state='acked',acked_at=1,ack_actor_seat_id='s',ack_generation=1,ack_observation='obs' WHERE message_id='m1';",
    );
    assert_eq!(fp(&store).pending_receipts, 0);

    // Invitation (a required invitation is the same pending invitation plus a
    // requirement episode, which does not move the count).
    let (store, conn, _) = fixture();
    base(&conn);
    changes(
        &store,
        &conn,
        "invitation",
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-s','t1','s',1,'pending',0,3,100,100);",
    );
    assert_eq!(fp(&store).other_pending, 1);

    // Warning open, then clear: a membership-interval warning about another
    // seat's invitation reaches every member of the thread; accepting the
    // invitation makes the condition non-actionable.
    let (store, conn, _) = fixture();
    base(&conn);
    conn.execute_batch(
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-o','t1','o',1,'pending',0,30,100,100);\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t4','s',1,1),('t4','o',1,1);\
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-member','i','t4',1,'warn','{}',0,12,1);",
    )
    .unwrap();
    changes(
        &store,
        &conn,
        "warning open",
        "INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('w-member',12,'t4',100,'o','invitation','inv-o');",
    );
    assert_eq!(fp(&store).other_pending, 1);
    changes(
        &store,
        &conn,
        "warning clear",
        "UPDATE invitations SET state='accepted',accepted_at=1,accepted_actor_seat_id='o',accepted_generation=1,accepted_observation='obs' WHERE id='inv-o';",
    );

    // Notice publication: an informational warning attributed to the seat.
    let (store, conn, _) = fixture();
    base(&conn);
    conn.execute_batch(
        "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t4','s',1,1);\
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-notice','i','t4',1,'warn','{}',0,12,1);",
    )
    .unwrap();
    changes(
        &store,
        &conn,
        "notice publication",
        "INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s','w-notice','t4',12,1);",
    );

    // Catch-up release: a pending receipt held by an active row is invisible;
    // ending the row releases it.
    let (store, conn, _) = fixture();
    base(&conn);
    conn.execute_batch(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m9','i','t2',2,'ordinary','b',0,6);\
         INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m9','t2','s','pending',100);\
         INSERT INTO catch_up(seat_id,thread_id,frontier_seq,binding_generation,execution_id,entered_at,state) VALUES ('s','t2',0,0,'e',0,'active');",
    )
    .unwrap();
    assert_eq!(fp(&store).pending_receipts, 0, "held by catch-up");
    changes(
        &store,
        &conn,
        "catch-up release",
        "UPDATE catch_up SET state='ended',end_reason='ready',ended_at=1,release_seq=50 WHERE seat_id='s';",
    );
    assert_eq!(fp(&store).pending_receipts, 1);

    // Attention version alone (the wake producers' bump) is a change.
    let (store, conn, _) = fixture();
    changes(
        &store,
        &conn,
        "attention version bump",
        "INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES ('s',1,1) ON CONFLICT(seat_id) DO UPDATE SET attention_version=attention_version+1;",
    );
    assert_eq!(view(&store).attention_version, 1);
}

/// ht-j16.34: another seat's overdue transition is an informational notice
/// for `s` (TRUST-POLICY A7). It stays pending in the digest (delivered at the
/// next check-in) but raises no mod attention: `other_pending` counts only
/// warnings that wake the seat, as native wake does. The affected seat `o`
/// counts its open condition until the clear.
#[test]
fn other_seat_transition_notice_is_not_mod_attention() {
    let (store, conn, _) = fixture();
    base(&conn);
    conn.execute_batch(
        "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t4','s',1,1),('t4','o',1,1);\
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-open','i','t4',1,'warn','{}',0,12,1),('w-clear','i','t4',2,'warn','{}',0,13,1);",
    )
    .unwrap();
    let before = fp(&store);
    {
        let turn = store.writer(&budget()).unwrap();
        turn.execute_batch(
            "INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES ('receipt','t4','c-o','o','w-open',12);\
             INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s','w-open','t4',12,1),('o','w-open','t4',12,1);",
        )
        .unwrap();
    }
    let digest = |seat: &str| {
        crate::store::attention::seat_digest(&conn, "i", &SeatId::new(seat), &|| Ok(()))
            .unwrap()
            .digest
            .warnings
            .count
    };
    assert_eq!(digest("s"), 1, "the notice is still delivered to s");
    assert_eq!(fp(&store), before, "another seat's open is not attention");
    let o = |store: &SqliteStore| {
        store
            .mod_seat_view(&SeatId::new("o"), &budget())
            .unwrap()
            .unwrap()
            .fingerprint
            .other_pending
    };
    assert_eq!(o(&store), 1, "the affected seat's own open is attention");
    {
        let turn = store.writer(&budget()).unwrap();
        turn.execute_batch(
            "UPDATE warning_conditions SET clear_warning_id='w-clear',cleared_seq=13 WHERE open_warning_id='w-open';\
             INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s','w-clear','t4',13,1),('o','w-clear','t4',13,1);",
        )
        .unwrap();
    }
    assert_eq!(digest("s"), 2, "the clear is delivered too");
    assert_eq!(fp(&store), before, "a clear is not attention");
    assert_eq!(
        o(&store),
        0,
        "a clear wakes nobody, the affected seat included"
    );
}

#[test]
fn lazy_row_changes_the_fingerprint_when_it_publishes_not_when_it_stages() {
    let (store, conn, _) = fixture();
    base(&conn);
    conn.execute_batch(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t2','s','joined');\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t2','s',1,1);\
         INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES('prep-1','i','scope','prep-1',zeroblob(32),'t2',0,0,0,0,0,0,0,'building','lazy');",
    )
    .unwrap();
    let staged_from = fp(&store);
    conn.execute(
        "INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','msg-1','t2','s')",
        [],
    )
    .unwrap();
    assert_eq!(fp(&store), staged_from, "a staged row is invisible");
    changes(
        &store,
        &conn,
        "lazy row publication",
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES('msg-1','i','t2',1,'ordinary',7,'passive',0,'lazy');\
         INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep-1','msg-1','i','t2',7,0,1,0,1,0);",
    );
    let published = fp(&store);
    assert_eq!(published.lazy_pending, 1);
    assert_eq!(published.max_lazy_rowid, 1);
    changes(
        &store,
        &conn,
        "lazy row displayed",
        "UPDATE lazy_recipients SET state='displayed';",
    );
    assert_eq!(fp(&store).lazy_pending, 0);
}

#[test]
fn fingerprint_changes_when_a_lifecycle_check_in_rebinds() {
    let (store, _conn, _) = fixture();
    check_in(
        &store,
        lifecycle(
            claude_claim("session-X", "00000000-0000-4000-8000-000000000001"),
            "op-1",
        ),
    )
    .unwrap();
    let first = fp(&store);
    assert_eq!(first.binding_generation, Some(1));
    // SessionStart of a new execution of the same seat: generation 2.
    let mut context = claude_claim("session-X", "00000000-0000-4000-8000-000000000002");
    context.binding_generation = 1;
    check_in(&store, lifecycle(context, "op-2")).unwrap();
    let second = view(&store);
    assert_ne!(first, second.fingerprint);
    assert_eq!(second.binding.as_ref().unwrap().generation, 2);
    assert_eq!(second.fingerprint.binding_generation, Some(2));
}

fn seed_stall_receipt(
    conn: &Connection,
    id: &str,
    sequence: i64,
    decision_seq: i64,
    decision_at: i64,
    body_len: usize,
) {
    conn.execute(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,'i','t2',?2,'ordinary',?3,?4,?5)",
        rusqlite::params![id, sequence, "x".repeat(body_len), decision_at, decision_seq],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t2','s','pending',100)",
        [id],
    )
    .unwrap();
}

#[test]
fn stall_oldest_excludes_truncated_and_later_publications() {
    let (store, conn, _) = fixture();
    base(&conn);
    let oldest = |bound: i64| {
        store
            .mod_stall_oldest(
                &seat_s(),
                UtcMillis(bound),
                WATCH_BODY_LIMIT_BYTES,
                &budget(),
            )
            .unwrap()
    };
    assert_eq!(oldest(10_000), None, "nothing pending");
    // The oldest receipt is truncated (one byte over the limit): never counts.
    seed_stall_receipt(&conn, "m-trunc", 1, 5, 100, WATCH_BODY_LIMIT_BYTES + 1);
    assert_eq!(oldest(10_000), None);
    // At exactly the limit it counts.
    seed_stall_receipt(&conn, "m-edge", 2, 6, 300, WATCH_BODY_LIMIT_BYTES);
    assert_eq!(oldest(10_000), Some(UtcMillis(300)));
    // A newer, small receipt does not displace the older one.
    seed_stall_receipt(&conn, "m-new", 3, 7, 900, 10);
    assert_eq!(oldest(10_000), Some(UtcMillis(300)));
    // Publications after the bound are excluded.
    assert_eq!(oldest(299), None);
    assert_eq!(oldest(300), Some(UtcMillis(300)));
    // A settled receipt no longer counts.
    conn.execute(
        "UPDATE receipts SET state='acked',acked_at=1,ack_actor_seat_id='s',ack_generation=1,ack_observation='obs' WHERE message_id='m-edge'",
        [],
    )
    .unwrap();
    assert_eq!(oldest(10_000), Some(UtcMillis(900)));
}

fn counting_observer(store: &SqliteStore) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&count);
    store.kicks.set_mod_observer(Arc::new(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    }));
    count
}

#[test]
fn commit_notifies_the_mod_observer_for_attention_sources_only() {
    let (store, _conn, _) = fixture();
    let count = counting_observer(&store);
    let write = |sql: &str| {
        let turn = store.writer(&budget()).unwrap();
        turn.execute_batch(sql).unwrap();
    };
    // A send to the seat: the attention producers' write.
    write(
        "INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES ('s',1,1) ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1;",
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    // A binding change.
    write("UPDATE seats SET generation=generation+1 WHERE id='s';");
    assert_eq!(count.load(Ordering::SeqCst), 2);
    // Only an unlisted table (a topic change) stays silent.
    write(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('tx','i','topic','goal',0,0);",
    );
    write("UPDATE threads SET topic='renamed' WHERE id='tx';");
    assert_eq!(count.load(Ordering::SeqCst), 2);
    // A rolled-back write never notifies.
    {
        let turn = store.writer(&budget()).unwrap();
        turn.execute_batch(
            "BEGIN; INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES ('s',1,1) ON CONFLICT(seat_id) DO UPDATE SET attention_version=attention_version+1; ROLLBACK;",
        )
        .unwrap();
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
    // A retention pass deleting settled rows never notifies.
    {
        let _origin = crate::service::kicks::enter_lane(crate::service::kicks::Lane::Retention);
        write("UPDATE wake_work SET attention_version=attention_version+1 WHERE seat_id='s';");
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
    // The next request-origin commit notifies again.
    write("UPDATE wake_work SET attention_version=attention_version+1 WHERE seat_id='s';");
    assert_eq!(count.load(Ordering::SeqCst), 3);
}

#[test]
fn lifecycle_check_in_through_the_store_notifies_the_observer() {
    let (store, _conn, _) = fixture();
    let count = counting_observer(&store);
    check_in(
        &store,
        lifecycle(
            claude_claim("session-X", "00000000-0000-4000-8000-000000000001"),
            "op-1",
        ),
    )
    .unwrap();
    assert!(
        count.load(Ordering::SeqCst) >= 1,
        "a binding change reaches the mod worker"
    );
}
