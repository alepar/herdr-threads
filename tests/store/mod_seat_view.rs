//! Mod watch store reads and the commit-hook notify (ht-j16.2). Included from
//! `src/store/seats.rs` as `mod_seat_view_tests`.
use super::*;
use crate::{
    ports::{ModBindingRef, ModFingerprint, ModStoreReads},
    protocol::{
        authority::Harness,
        ids::{BindingId, NativeSessionId},
        time::UtcMillis,
        watch::WATCH_BODY_LIMIT_BYTES,
    },
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

/// The view of the seat's oldest open top-level binding (a binding that matches nothing when the
/// seat has none: the seat-level parts are the same).
fn view(store: &SqliteStore) -> crate::ports::ModSeatView {
    let db = store.context.open_query(budget()).unwrap();
    let first = crate::store::bindings::list_open_top_level(&db, &seat_s())
        .unwrap()
        .into_iter()
        .next()
        .unwrap_or_else(|| BindingId::new("no-such-binding"));
    view_of(store, &first)
}

fn view_of(store: &SqliteStore, binding: &BindingId) -> crate::ports::ModSeatView {
    store
        .mod_binding_view(&seat_s(), ModBindingRef::Id(binding), &budget())
        .unwrap()
        .unwrap()
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
    assert_eq!(result.context.binding_generation, 0);
    let after = view(&store);
    let held = after.binding.clone().unwrap();
    assert_eq!(held.generation, 0);
    assert_eq!(held.provenance, "cooperative_top_level");
    assert_eq!(held.harness, "claude");
    assert_eq!(held.native_session, "session-X");
    assert!(held.holds_channel() && held.top_level && held.connected && held.ended_at.is_none());
    assert_eq!(after.fingerprint.binding_generation, Some(0));
    assert_eq!(after.state, "resolved");
    // An unknown seat has no view.
    assert_eq!(
        store
            .mod_binding_view(
                &SeatId::new("nobody"),
                ModBindingRef::Id(&held.binding_id),
                &budget()
            )
            .unwrap(),
        None
    );
}

#[test]
fn view_reports_held_unresolved_retired() {
    let (store, conn, _) = fixture();
    assert!(!view(&store).held);
    conn.execute(
        "INSERT INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','b',1,'hold')",
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
        "UPDATE local_node SET decision_seq=100;\
         INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('o','34750f98bd59fcfc946da45aaabe933b','resolved','native','po',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));\
         INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('t1','34750f98bd59fcfc946da45aaabe933b','a','g',0,0,0),('t2','34750f98bd59fcfc946da45aaabe933b','b','g',0,0,0),('t4','34750f98bd59fcfc946da45aaabe933b','d','g',0,0,0);",
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
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m1','34750f98bd59fcfc946da45aaabe933b','t2',1,'ordinary','b',0,5);\
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
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,origin_node) VALUES ('inv-s','t1','s',1,'pending',0,3,100,100,(SELECT id FROM local_node));",
    );
    assert_eq!(fp(&store).other_pending, 1);

    // Warning open, then clear: a membership-interval warning about another
    // seat's invitation reaches every member of the thread; accepting the
    // invitation makes the condition non-actionable.
    let (store, conn, _) = fixture();
    base(&conn);
    conn.execute_batch(
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,origin_node) VALUES ('inv-o','t1','o',1,'pending',0,30,100,100,(SELECT id FROM local_node));\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t4','s',1,1),('t4','o',1,1);\
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-member','34750f98bd59fcfc946da45aaabe933b','t4',1,'warn','{}',0,12,1);",
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
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-notice','34750f98bd59fcfc946da45aaabe933b','t4',1,'warn','{}',0,12,1);",
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
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m9','34750f98bd59fcfc946da45aaabe933b','t2',2,'ordinary','b',0,6);\
         INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m9','t2','s','pending',100);\
         INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) SELECT 's',1,'p','b',0,'codex','catch-up-row','e','cooperative_top_level',0,'tm','inc' WHERE NOT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL); INSERT INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,state) SELECT binding_id,'s','t2',0,0,'active' FROM occupant_bindings WHERE seat_id='s';",
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
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-open','34750f98bd59fcfc946da45aaabe933b','t4',1,'warn','{}',0,12,1),('w-clear','34750f98bd59fcfc946da45aaabe933b','t4',2,'warn','{}',0,13,1);",
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
        crate::store::attention::seat_digest(
            &conn,
            "34750f98bd59fcfc946da45aaabe933b",
            &SeatId::new(seat),
            &|| Ok(()),
        )
        .unwrap()
        .digest
        .warnings
        .count
    };
    assert_eq!(digest("s"), 1, "the notice is still delivered to s");
    assert_eq!(fp(&store), before, "another seat's open is not attention");
    let o = |store: &SqliteStore| {
        store
            .mod_binding_view(
                &SeatId::new("o"),
                ModBindingRef::Id(&BindingId::new("none")),
                &budget(),
            )
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
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',0,'p','b',0,'codex','lazy-session','e','cooperative_top_level',0,'tm','inc');\
         INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t2','s','joined');\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t2','s',1,1);\
         INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES('prep-1','34750f98bd59fcfc946da45aaabe933b','scope','prep-1',zeroblob(32),'t2',0,0,0,0,0,0,0,'building','lazy',1);",
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
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES('msg-1','34750f98bd59fcfc946da45aaabe933b','t2',1,'ordinary',7,'passive',0,'lazy',1);\
         INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep-1','msg-1','34750f98bd59fcfc946da45aaabe933b','t2',7,0,1,0,1,0);",
    );
    let published = fp(&store);
    assert_eq!(published.lazy_pending, 1);
    assert_eq!(published.max_lazy_rowid, 1);
    changes(
        &store,
        &conn,
        "lazy row displayed (the binding's cursor passes it)",
        "INSERT INTO binding_cursors(binding_id,thread_id,seq) SELECT binding_id,'t2',1 FROM occupant_bindings WHERE seat_id='s'; UPDATE lazy_recipients SET legacy_pending=legacy_pending;",
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
    assert_eq!(first.binding_generation, Some(0));
    // SessionStart of a new execution of the same session (ht-54r.3: the binding is
    // re-registered in place and the seat's mapping generation stays 0).
    let mut context = claude_claim("session-X", "00000000-0000-4000-8000-000000000002");
    context.binding_generation = 0;
    check_in(&store, lifecycle(context, "op-2")).unwrap();
    let second = view(&store);
    assert_eq!(second.binding.as_ref().unwrap().generation, 0);
    assert_eq!(second.fingerprint.binding_generation, Some(0));
}

/// An ordinary message `id` (sequence `seq` of thread `t2`) published to seat `s` as a recipient.
fn deliver(conn: &Connection, id: &str, seq: u32) {
    conn.execute_batch(&format!(
        "INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','{id}','t2',{seq},'ordinary','b',{seq},{seq}) ON CONFLICT DO NOTHING;
         INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p-{id}','34750f98bd59fcfc946da45aaabe933b','actor','{id}',zeroblob(32),'t2',0,0,0,0,0,0,1,'sealed');
         INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p-{id}','t2','s',1,300,0);
         INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) SELECT 'p-{id}','{id}','34750f98bd59fcfc946da45aaabe933b','t2',decision_seq,{seq},{seq},0,1,0 FROM messages WHERE id='{id}';"
    ))
    .unwrap();
}

/// Two Claude sessions of seat `s` (two top-level bindings), `s` joined to thread `t2`.
fn two_bindings(store: &SqliteStore, conn: &Connection) -> (BindingId, BindingId) {
    base(conn);
    conn.execute_batch(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t2','s','joined');\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t2','s',1,1);",
    )
    .unwrap();
    let mut ids = Vec::new();
    for (n, (session, execution, operation)) in [
        ("session-A", "00000000-0000-4000-8000-00000000000a", "op-a"),
        ("session-B", "00000000-0000-4000-8000-00000000000b", "op-b"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut context = claude_claim(session, execution);
        context.binding_generation = 0;
        // Distinct processes: one process never holds two bindings.
        context.liveness = Some(crate::protocol::authority::LivenessFingerprint {
            pid: 10 + n as u32,
            start: 10 + n as u64,
        });
        check_in(store, lifecycle(context, operation)).unwrap();
        let id: String = conn
            .query_row(
                "SELECT binding_id FROM occupant_bindings WHERE native_session=?1",
                [session],
                |r| r.get(0),
            )
            .unwrap();
        ids.push(BindingId::new(id));
    }
    (ids.remove(0), ids.remove(0))
}

#[test]
fn binding_view_fingerprint_moves_with_that_bindings_unread_only() {
    let (store, conn, _) = fixture();
    let (a, b) = two_bindings(&store, &conn);
    // Both bindings carry no cursor for t2 (joined after they enrolled): mail is unread by both.
    deliver(&conn, "m1", 1);
    deliver(&conn, "m2", 2);
    let (a0, b0) = (view_of(&store, &a), view_of(&store, &b));
    assert_eq!(a0.binding.as_ref().unwrap().binding_id, a);
    assert_eq!(b0.binding.as_ref().unwrap().binding_id, b);
    assert_eq!(a0.fingerprint.unread, 2);
    assert_eq!(b0.fingerprint.unread, 2);
    assert_eq!(
        a0.fingerprint, b0.fingerprint,
        "two bindings with the same unread and the same seat agree"
    );
    // A reads m1 (its cursor passes it): A's fingerprint moves, B's does not.
    conn.execute(
        "UPDATE binding_cursors SET seq=1 WHERE binding_id=?1 AND thread_id='t2'",
        [a.as_str()],
    )
    .unwrap();
    let (a1, b1) = (view_of(&store, &a), view_of(&store, &b));
    assert_ne!(a1.fingerprint, a0.fingerprint);
    assert_eq!(a1.fingerprint.unread, 1);
    assert_eq!(b1.fingerprint, b0.fingerprint, "A's cursor is not B's");
    // A reads everything: nothing is unread for A, while B still has its two.
    conn.execute(
        "UPDATE binding_cursors SET seq=2 WHERE binding_id=?1",
        [a.as_str()],
    )
    .unwrap();
    let (a2, b2) = (view_of(&store, &a), view_of(&store, &b));
    assert_eq!(a2.fingerprint.unread, 0);
    assert_eq!(b2.fingerprint.unread, 2);
    // New mail moves both again.
    deliver(&conn, "m3", 3);
    assert_ne!(view_of(&store, &a).fingerprint, a2.fingerprint);
    assert_ne!(view_of(&store, &b).fingerprint, b2.fingerprint);
}

#[test]
fn ended_or_disconnected_binding_view_reports_it() {
    use crate::store::bindings::{DisconnectReason, disconnect};
    let (store, conn, _) = fixture();
    let (a, b) = two_bindings(&store, &conn);
    deliver(&conn, "m1", 1);
    assert!(view_of(&store, &a).binding.unwrap().holds_channel());
    assert_eq!(view_of(&store, &a).fingerprint.unread, 1);
    // A disconnects: still open, no longer able to hold a channel, and no unread of its own.
    assert!(disconnect(&conn, &a, DisconnectReason::ProcessExit, UtcMillis(5)).unwrap());
    let disconnected = view_of(&store, &a);
    let held = disconnected.binding.clone().unwrap();
    assert!(!held.connected && held.ended_at.is_none() && !held.holds_channel());
    assert_eq!(disconnected.fingerprint.unread, 0);
    // B is untouched by A's disconnect.
    let sibling = view_of(&store, &b);
    assert!(sibling.binding.unwrap().holds_channel());
    assert_eq!(sibling.fingerprint.unread, 1);
    // A ends: the id reference still reports it, with its end time; the claim reference (open
    // bindings only) resolves nothing.
    conn.execute(
        "UPDATE occupant_bindings SET ended_at=9 WHERE binding_id=?1",
        [a.as_str()],
    )
    .unwrap();
    let ended = view_of(&store, &a).binding.unwrap();
    assert_eq!(ended.ended_at, Some(UtcMillis(9)));
    assert!(!ended.holds_channel());
    let mut stale = claude_claim("session-A", "00000000-0000-4000-8000-00000000000a");
    stale.binding_generation = 0;
    let by_claim = store
        .mod_binding_view(&seat_s(), ModBindingRef::Claim(&stale), &budget())
        .unwrap()
        .unwrap();
    assert_eq!(by_claim.binding, None);
    // A claim names a binding of its own seat only, and by its key: B resolves, a stranger does
    // not, and an agent id names a different (subagent) key.
    let mut b_claim = claude_claim("session-B", "00000000-0000-4000-8000-00000000000b");
    b_claim.binding_generation = 0;
    let resolved = store
        .mod_binding_view(&seat_s(), ModBindingRef::Claim(&b_claim), &budget())
        .unwrap()
        .unwrap();
    assert_eq!(resolved.binding.unwrap().binding_id, b);
    b_claim.agent_id = Some(crate::protocol::ids::AgentId::new("agent-1"));
    assert_eq!(
        store
            .mod_binding_view(&seat_s(), ModBindingRef::Claim(&b_claim), &budget())
            .unwrap()
            .unwrap()
            .binding,
        None
    );
    let other = store.mod_binding_view(&SeatId::new("o"), ModBindingRef::Id(&b), &budget());
    assert!(
        other.unwrap().unwrap().binding.is_none(),
        "another seat's binding is not this seat's"
    );
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
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','t2',?2,'ordinary',?3,?4,?5)",
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

/// A published pending delivery row (message `id`, sequence `n` of `t2`) for seat `s` with `priority`
/// and no receipt (D9: a bare p0 send makes a delivery row only).
fn seed_stall_row(
    conn: &Connection,
    n: i64,
    priority: i64,
    decision_seq: i64,
    decision_at: i64,
    body_len: usize,
) {
    conn.execute(
        "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','scope',?1,zeroblob(32),'t2',0,0,0,0,0,0,0,'building','lazy',?2)",
        rusqlite::params![format!("prep-{n}"), priority],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES(?1,?2,'t2','s',?3)",
        rusqlite::params![format!("prep-{n}"), format!("msg-{n}"), priority],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','t2',?2,'ordinary',?3,?4,?5,'lazy',?6)",
        rusqlite::params![format!("msg-{n}"), n, decision_seq, "x".repeat(body_len), decision_at, priority],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES(?1,?2,'34750f98bd59fcfc946da45aaabe933b','t2',?3,?4,?5,0,1,0)",
        rusqlite::params![format!("prep-{n}"), format!("msg-{n}"), decision_seq, decision_at, n],
    )
    .unwrap();
    // Publication (`note_published`) stamps the recipient row with the manifest's decision sequence.
    conn.execute(
        "UPDATE lazy_recipients SET decision_seq=?2 WHERE preparation_id=?1",
        rusqlite::params![format!("prep-{n}"), decision_seq],
    )
    .unwrap();
}

#[test]
fn stall_oldest_counts_threshold_admitted_delivery_rows() {
    let (store, conn, _) = fixture();
    base(&conn);
    conn.execute_batch(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',0,'p','b',0,'claude','stall-session','e','cooperative_top_level',0,'tm','inc');\
         INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t2','s','joined');\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t2','s',1,1);",
    )
    .unwrap();
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
    // A p1 row at the default threshold T = 0 is not admitted.
    seed_stall_row(&conn, 1, 1, 5, 200, 10);
    assert_eq!(oldest(10_000), None);
    // A p0 row is admitted; a truncated older p0 row never counts.
    seed_stall_row(&conn, 2, 0, 6, 400, 10);
    seed_stall_row(&conn, 3, 0, 7, 100, WATCH_BODY_LIMIT_BYTES + 1);
    assert_eq!(oldest(10_000), Some(UtcMillis(400)));
    assert_eq!(oldest(399), None, "published after the bound");
    // T < 0 admits the p1 row too.
    conn.execute("UPDATE seats SET nudge_threshold=-1 WHERE id='s'", [])
        .unwrap();
    assert_eq!(oldest(10_000), Some(UtcMillis(200)));
    conn.execute("UPDATE seats SET nudge_threshold=0 WHERE id='s'", [])
        .unwrap();
    assert_eq!(oldest(10_000), Some(UtcMillis(400)));
    // A pending receipt older than every row wins by minimum.
    seed_stall_receipt(&conn, "m-rcpt", 9, 9, 50, 10);
    assert_eq!(oldest(10_000), Some(UtcMillis(50)));
    conn.execute("UPDATE receipts SET state='acked',acked_at=1,ack_actor_seat_id='s',ack_generation=1,ack_observation='obs' WHERE message_id='m-rcpt'", [])
        .unwrap();
    assert_eq!(oldest(10_000), Some(UtcMillis(400)));
    // The binding's cursor passes the rows: nothing is unread for a connected binding.
    conn.execute(
        "INSERT OR REPLACE INTO binding_cursors(binding_id,thread_id,seq) SELECT binding_id,'t2',100 FROM occupant_bindings WHERE seat_id='s'",
        [],
    )
    .unwrap();
    assert_eq!(oldest(10_000), None);
}

/// ht-yha.54 (S1): the stall rule reads each connected binding's own pending view, never the seat-wide
/// `state`. `resumed` was away (end event) while `live` displayed the p0 row; once it reconnects its
/// re-seeded view makes the row unread again. Kills: `r.state='pending'` in the stall query.
#[test]
fn stall_oldest_sees_a_reconnected_bindings_pending_row() {
    use crate::store::bindings::{ConnectTrigger, DisconnectReason};
    let (store, conn, _) = fixture();
    base(&conn);
    // A join writes a cursor per open binding (`read_cursors::init_for_join`); the reconnect
    // re-seed walks only past cursors (ht-yha.67).
    conn.execute_batch(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation,binding_id) VALUES ('s',0,'p','b',0,'claude','stall-live','e','cooperative_top_level',0,'tm','inc','live');\
         INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation,binding_id) VALUES ('s',0,'p','b',0,'claude','stall-resumed','e','cooperative_top_level',0,'tm','inc','resumed');\
         INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t2','s','joined');\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t2','s',1,1);\
         INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES ('live','t2',0),('resumed','t2',0);",
    )
    .unwrap();
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
    seed_stall_row(&conn, 2, 0, 6, 400, 10);
    assert_eq!(oldest(10_000), Some(UtcMillis(400)));
    assert!(
        crate::store::bindings::disconnect_with_hooks(
            &conn,
            &BindingId::new("resumed"),
            DisconnectReason::EndEvent,
            UtcMillis(5),
            &[]
        )
        .unwrap()
    );
    conn.execute_batch(
        "INSERT OR REPLACE INTO binding_cursors(binding_id,thread_id,seq) VALUES ('live','t2',100);\
         UPDATE lazy_recipients SET state='displayed' WHERE message_id='msg-2';",
    )
    .unwrap();
    assert_eq!(oldest(10_000), None, "no connected binding finds it unread");
    assert!(
        crate::store::bindings::connect(
            &conn,
            &BindingId::new("resumed"),
            None,
            ConnectTrigger::Lifecycle,
            UtcMillis(6)
        )
        .unwrap()
    );
    assert_eq!(oldest(10_000), Some(UtcMillis(400)));
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
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('tx','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0);",
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

/// ht-poh.53: the hook-pull notice probe is in the view but deliberately not
/// in the fingerprint (notices never wake and never move mod attention).
#[test]
fn view_reports_the_newest_pending_notice_outside_the_fingerprint() {
    let (store, conn, _) = fixture();
    base(&conn);
    conn.execute_batch(
        "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t4','s',1,1),('t4','o',1,1);\
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-1','34750f98bd59fcfc946da45aaabe933b','t4',1,'warn','{}',0,12,1),('w-2','34750f98bd59fcfc946da45aaabe933b','t4',2,'warn','{}',0,13,1);\
         INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES ('receipt','t4','c-1','o','w-1',12),('receipt','t4','c-2','o','w-2',13);",
    )
    .unwrap();
    assert_eq!(view(&store).newest_pending_notice, None);
    let before = fp(&store);
    conn.execute_batch(
        "INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s','w-1','t4',12,1);",
    )
    .unwrap();
    let first = view(&store).newest_pending_notice;
    assert!(first.is_some());
    conn.execute_batch(
        "INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s','w-2','t4',13,1);",
    )
    .unwrap();
    let second = view(&store).newest_pending_notice;
    assert!(second > first, "{first:?} {second:?}");
    assert_eq!(fp(&store), before, "notices are not in the fingerprint");
}
