//! `DeadlineWindow` (ht-yha.5.3): window-level behaviour driven by persisted walk positions.
use super::*;
use crate::store::schema;

const DECISION: u64 = 1_000;

fn db(bound: bool) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    db.execute_batch(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1000);\
         INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES ('s','i','resolved','native','ps',1,1,0,1);\
         INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','ps','b',1,1,0,'fresh','term-ps','inc','coherent_enumeration',1);\
         INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,1);\
         INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','joined');\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s',1,1);",
    )
    .unwrap();
    if bound {
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,'ps','b',1,1,'codex','n','00000000-0000-4000-8000-0000000000aa','cooperative_top_level',0,0,'term-ps','inc')", []).unwrap();
        crate::store::read_cursors::init_for_new_binding(&db, &binding(&db)).unwrap();
    }
    db
}

fn binding(db: &Connection) -> BindingId {
    BindingId::new(
        db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='s'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
}

fn seat() -> SeatId {
    SeatId::new("s")
}

/// A physical receipt on its own message: `state` pending or acked, due at `deadline`.
fn receipt(db: &Connection, n: i64, deadline: i64, state: &str) {
    db.execute(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary','b',0,?2)",
        rusqlite::params![format!("r{n}"), n],
    )
    .unwrap();
    db.execute(
        "INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'t','s','pending',100,0,?2)",
        rusqlite::params![format!("r{n}"), deadline],
    )
    .unwrap();
    if state == "acked" {
        db.execute("UPDATE receipts SET state='acked',ack_actor_seat_id='s',ack_generation=1,ack_observation='o',acked_at=1 WHERE message_id=?1", [format!("r{n}")]).unwrap();
    }
}

fn invitation(
    db: &Connection,
    name: &str,
    episode: i64,
    confirmation: Option<&str>,
    deadline: i64,
) {
    db.execute(
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,confirmation,inviter_seat_id) VALUES (?1,'t','s',?2,'pending',0,5,?3,100,?4,?5)",
        rusqlite::params![name, episode, deadline, confirmation, confirmation.map(|_| "s")],
    )
    .unwrap();
}

fn cursor(db: &Connection, bound: bool) -> DeadlineCursor {
    let max = |sql: &str| -> i64 { db.query_row(sql, [], |r| r.get(0)).unwrap() };
    DeadlineCursor {
        high: DeadlineHighWaters {
            invitation: max("SELECT coalesce(max(ordinal),0) FROM invitations"),
            unread: bound
                .then(|| max("SELECT coalesce(max(ordinal),0) FROM prepared_recipients") as u64),
            physical: max("SELECT coalesce(max(ordinal),0) FROM receipts"),
            manifest: max("SELECT coalesce(max(ordinal),0) FROM prepared_recipients"),
        },
        window_start: DeadlineWalks {
            invitation_after: 0,
            unread_after: bound.then_some(0),
            physical_after: 0,
            manifest_after: 0,
            next_manifest: false,
        },
        window_end: None,
        done: false,
    }
}

fn collect(
    db: &Connection,
    bound: bool,
    cursor: &DeadlineCursor,
    last: Option<&DeliveryOrderKey>,
    visits: &mut usize,
) -> DeadlineWindow {
    let reading = bound.then(|| binding(db));
    DeadlineWindow::collect(
        db,
        &seat(),
        reading.as_ref(),
        true,
        cursor,
        DECISION,
        last,
        visits,
    )
    .unwrap()
}

fn label(item: &DeadlineItem) -> String {
    match &item.item {
        DeadlineRef::Invitation { id, .. } => id.as_str().to_owned(),
        DeadlineRef::Unread { message, .. } | DeadlineRef::Receipt { message, .. } => {
            message.as_str().to_owned()
        }
    }
}

/// Emit up to `count` items from the window head, returning their labels and the last key.
fn emit(window: &mut DeadlineWindow, count: usize) -> (Vec<String>, Option<DeliveryOrderKey>) {
    let mut out = Vec::new();
    let mut last = None;
    while out.len() < count {
        let Some(head) = window.head() else { break };
        out.push(label(head));
        last = Some(head.key);
        window.advance();
    }
    (out, last)
}

#[test]
fn window_orders_exactly_when_backlog_fits() {
    let db = db(true);
    receipt(&db, 1, 300, "pending");
    receipt(&db, 2, 100, "pending");
    receipt(&db, 3, 200, "pending");
    invitation(&db, "inv-ack", 1, Some("require_ack"), 150);
    invitation(&db, "inv-lazy", 2, Some("lazy"), 50);
    let cursor = cursor(&db, true);
    let mut visits = 0;
    let mut window = collect(&db, true, &cursor, None, &mut visits);
    assert!(!window.done());
    let (order, _) = emit(&mut window, 10);
    assert_eq!(order, ["r2", "inv-ack", "r3", "r1", "inv-lazy"]);
    assert!(window.done());
    assert_eq!(visits, 5, "2 invitations + 3 receipts");
}

#[test]
fn seventy_receipts_exact_within_each_window_nothing_lost_or_repeated() {
    let db = db(false);
    // Deadlines are a permutation of 1..=70 (37 is coprime with 70).
    for n in 1..=70 {
        receipt(&db, n, (n * 37) % 70 + 1, "pending");
    }
    let mut cursor = cursor(&db, false);
    let mut visits = 0;
    let mut window = collect(&db, false, &cursor, None, &mut visits);
    let deadlines: Vec<i64> = window
        .items
        .iter()
        .map(|i| i.key.deadline.unwrap().0)
        .collect();
    // One visit yields at most one item, so the visit share closes the window before 64 items.
    let size = DEADLINE_WINDOW.min(WINDOW_VISITS);
    assert_eq!(deadlines.len(), size);
    assert!(deadlines.windows(2).all(|w| w[0] < w[1]), "sorted");
    let (mut seen, last) = emit(&mut window, 10);
    window.persist(&mut cursor);
    assert!(cursor.window_end.is_some());
    assert_eq!(cursor.window_start.physical_after, 0);

    let mut window = collect(&db, false, &cursor, last.as_ref(), &mut visits);
    assert_eq!(window.items.len(), size - 10);
    assert!(window.items.iter().all(|i| i.key > last.unwrap()));
    let (rest, _) = emit(&mut window, 100);
    seen.extend(rest);
    window.persist(&mut cursor);
    assert!(cursor.window_end.is_none());
    assert!(!cursor.done);
    assert_eq!(cursor.window_start.physical_after, size as i64);

    let mut window = collect(&db, false, &cursor, None, &mut visits);
    assert_eq!(window.items.len(), 70 - size);
    let (tail, _) = emit(&mut window, 100);
    seen.extend(tail);
    assert!(window.done());
    window.persist(&mut cursor);
    assert!(cursor.done);

    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(seen.len(), 70);
    assert_eq!(unique.len(), 70, "no repeats");
}

#[test]
fn window_closes_at_its_visit_share_and_every_page_advances() {
    let db = db(false);
    for n in 1..=200 {
        receipt(&db, n, n, "acked");
    }
    for n in 201..=203 {
        receipt(&db, n, n, "pending");
    }
    let mut cursor = cursor(&db, false);
    let mut found = Vec::new();
    let mut rounds = 0;
    while !cursor.done {
        rounds += 1;
        assert!(rounds <= 6, "bounded rounds");
        let before = cursor.window_start.physical_after;
        let mut visits = 0;
        let mut window = collect(&db, false, &cursor, None, &mut visits);
        assert!(visits <= WINDOW_VISITS, "visits {visits}");
        assert!(visits > 0);
        let (items, _) = emit(&mut window, 100);
        found.extend(items);
        window.persist(&mut cursor);
        assert!(
            cursor.window_start.physical_after > before,
            "window_start advances"
        );
    }
    assert_eq!(found, ["r201", "r202", "r203"]);
    assert_eq!(rounds, 5, "200 settled / 50 visits, then the pending tail");
}

#[test]
fn deadline_less_legacy_invitation_sorts_after_deadlined_p0_items() {
    let db = db(false);
    invitation(&db, "inv-legacy", 1, None, 10);
    receipt(&db, 1, 9_000, "pending");
    let cursor = cursor(&db, false);
    let mut window = collect(&db, false, &cursor, None, &mut 0);
    assert_eq!(window.items[1].key.deadline, None);
    let (order, _) = emit(&mut window, 10);
    assert_eq!(order, ["r1", "inv-legacy"]);
}

#[test]
fn receipt_unread_dedupe_preserved() {
    let db = db(true);
    db.execute_batch(
        "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');\
         INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p','t','s',1,300,0);\
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('u1','i','t',1,'ordinary','s','b',7000,10);\
         INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('p','u1','i','t',10,7000,1,0,1,0);",
    )
    .unwrap();
    receipt(&db, 50, 400, "pending");
    let cursor = cursor(&db, true);
    let mut window = collect(&db, true, &cursor, None, &mut 0);
    let (order, _) = emit(&mut window, 10);
    assert_eq!(order, ["r50", "u1"], "the manifest receipt is listed once");
    assert!(matches!(
        window.items.iter().find(|i| label(i) == "u1").unwrap().item,
        DeadlineRef::Unread { ack: true, .. }
    ));
    assert_eq!(window.items.iter().filter(|i| label(i) == "u1").count(), 1);
}

#[test]
fn fresh_window_skips_nothing() {
    let db = db(false);
    receipt(&db, 1, 100, "pending");
    receipt(&db, 2, 200, "pending");
    let cursor = cursor(&db, false);
    let high = DeliveryOrderKey {
        priority: DeliveryPriority::p0(),
        deadline: Some(UtcMillis(150)),
        decision_seq: u64::MAX,
        rank: SourceRank::DeliveryRow,
        ordinal: u64::MAX,
    };
    let fresh = collect(&db, false, &cursor, Some(&high), &mut 0);
    assert_eq!(fresh.items.len(), 2, "a fresh window skips nothing");
    let mut recollect = cursor;
    recollect.window_end = Some(fresh.end);
    let again = collect(&db, false, &recollect, Some(&high), &mut 0);
    assert_eq!(
        again.items.iter().map(label).collect::<Vec<_>>(),
        ["r2"],
        "a re-collected window skips keys <= last"
    );
}
