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
        &mut None,
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
fn settled_physical_receipts_cost_no_visit() {
    let db = db(false);
    for n in 1..=200 {
        receipt(&db, n, n, "acked");
    }
    for n in 201..=203 {
        receipt(&db, n, n, "pending");
    }
    let mut cursor = cursor(&db, false);
    let mut visits = 0;
    let mut window = collect(&db, false, &cursor, None, &mut visits);
    assert_eq!(visits, 3, "only the three pending receipts are visited");
    let (items, _) = emit(&mut window, 100);
    assert_eq!(items, ["r201", "r202", "r203"]);
    window.persist(&mut cursor);
    assert!(cursor.done, "one round, not five");
}

/// A manifest receipt for seat `s` on its own message `u{n}` (sequence `seq`): preparation `p{n}`
/// with one prepared recipient; published (manifest row) unless `published` is false; settled
/// `acked` when `acked`.
fn manifest_receipt(db: &Connection, n: i64, seq: i64, published: bool, acked: bool) {
    db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES (?1,'i','actor',?2,zeroblob(32),'t',0,0,0,0,0,0,1,'sealed')",
        rusqlite::params![format!("p{n}"), format!("op{n}")]).unwrap();
    db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES (?1,'t','s',1,300,0)",
        [format!("p{n}")]).unwrap();
    if !published {
        return;
    }
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary','s','b',7000,?2)",
        rusqlite::params![format!("u{n}"), seq]).unwrap();
    db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES (?1,?2,'i','t',?3,7000,?3,0,1,0)",
        rusqlite::params![format!("p{n}"), format!("u{n}"), seq]).unwrap();
    if acked {
        db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES (?1,'s','acked','s',1,'o',1) ON CONFLICT(message_id,seat_id) DO UPDATE SET state='acked',ack_actor_seat_id='s',ack_generation=1,ack_observation='o',acked_at=1",
            [format!("u{n}")]).unwrap();
    }
}

#[test]
fn unpublished_preparations_cost_no_visit() {
    let db = db(false);
    for n in 1..=150 {
        manifest_receipt(&db, n, n, false, false);
    }
    manifest_receipt(&db, 151, 151, true, false);
    let cursor = cursor(&db, false);
    let mut visits = 0;
    let window = collect(&db, false, &cursor, None, &mut visits);
    assert_eq!(window.items.iter().map(label).collect::<Vec<_>>(), ["u151"]);
    assert_eq!(visits, 1);
}

#[test]
fn settled_manifest_receipts_cost_no_visit() {
    let db = db(false);
    for n in 1..=200 {
        manifest_receipt(&db, n, n, true, true);
    }
    manifest_receipt(&db, 201, 201, true, false);
    let cursor = cursor(&db, false);
    let mut visits = 0;
    let window = collect(&db, false, &cursor, None, &mut visits);
    assert_eq!(window.items.iter().map(label).collect::<Vec<_>>(), ["u201"]);
    assert_eq!(visits, 1);
}

#[test]
fn read_settled_history_costs_the_unread_walk_no_visit() {
    let db = db(true);
    for n in 1..=200 {
        manifest_receipt(&db, n, n, true, true);
    }
    db.execute("INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES (?1,'t',200) ON CONFLICT(binding_id,thread_id) DO UPDATE SET seq=excluded.seq", [binding(&db).as_str()]).unwrap();
    manifest_receipt(&db, 201, 201, true, false);
    let cursor = cursor(&db, true);
    let mut visits = 0;
    let window = collect(&db, true, &cursor, None, &mut visits);
    assert_eq!(window.items.iter().map(label).collect::<Vec<_>>(), ["u201"]);
    assert!(matches!(
        window.items[0].item,
        DeadlineRef::Unread { ack: true, .. }
    ));
    assert_eq!(visits, 2, "one unread visit and one deduped receipt visit");
}

#[test]
fn settled_invitations_cost_no_visit() {
    let db = db(false);
    for e in 1..=200 {
        let name = format!("inv{e}");
        invitation(&db, &name, e, Some("require_ack"), 100);
        db.execute("UPDATE invitations SET state='accepted',accepted_at=1,accepted_actor_seat_id='s',accepted_generation=1,accepted_observation='o' WHERE id=?1", [&name]).unwrap();
    }
    invitation(&db, "inv201", 201, Some("require_ack"), 100);
    let cursor = cursor(&db, false);
    let mut visits = 0;
    let window = collect(&db, false, &cursor, None, &mut visits);
    assert_eq!(
        window.items.iter().map(label).collect::<Vec<_>>(),
        ["inv201"]
    );
    assert_eq!(visits, 1);
}

#[test]
fn seat_pending_scope_lists_the_seat_scope_pending_set() {
    use crate::store::effective::{
        EffectiveReceiptState, ReceiptScanScope, ReceiptSource, scan_effective_receipts,
    };
    let db = db(false);
    for n in 1..=5 {
        receipt(&db, n, n, "pending");
        receipt(&db, 10 + n, n, "acked");
        manifest_receipt(&db, 20 + n, 100 + n, true, false);
        manifest_receipt(&db, 30 + n, 200 + n, true, true);
        manifest_receipt(&db, 40 + n, 300 + n, false, false);
    }
    let pending = |scope: ReceiptScanScope| -> (Vec<(ReceiptSource, String)>, usize) {
        let (mut position, mut out, mut total) = (None, Vec::new(), 0usize);
        loop {
            let slice = scan_effective_receipts(&db, &scope, position, 1).unwrap();
            total += slice.visited as usize;
            out.extend(
                slice
                    .items
                    .into_iter()
                    .filter(|r| r.state == EffectiveReceiptState::Pending)
                    .map(|r| (r.source, r.message_id)),
            );
            position = Some(slice.position);
            if slice.visited == 0 {
                break;
            }
        }
        out.sort_by_key(|(source, id)| (*source == ReceiptSource::Manifest, id.clone()));
        (out, total)
    };
    let (seat_set, _) = pending(ReceiptScanScope::Seat("s".into()));
    let (pending_set, visited) = pending(ReceiptScanScope::SeatPending("s".into()));
    assert_eq!(seat_set.len(), 10);
    assert_eq!(pending_set, seat_set);
    assert_eq!(visited, 10, "only pending, published rows are visited");
}

#[test]
fn seat_pending_manifest_seek_drives_from_the_projection() {
    let db = db(false);
    let plan: Vec<String> = db
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            crate::store::effective::SEAT_PENDING_MANIFEST_SQL
        ))
        .unwrap()
        .query_map(rusqlite::params!["s", 0, 1], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        plan[0].contains("digest_pending_manifest_receipts")
            && plan[0].contains("digest_pending_manifest_receipts_seat"),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|row| row.contains("send_manifests") && row.contains("preparation_id=?")),
        "{plan:?}"
    );
    assert!(
        !plan
            .iter()
            .any(|row| row.starts_with("SCAN") && row.contains("send_manifests")),
        "{plan:?}"
    );
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

mod page_level {
    //! The first inbox and hook page of a seat with long settled history and a staged backlog.
    use crate::ports::{DurableWorkAdmission, SendPreparationProgress};
    use crate::protocol::{
        authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
        commands::{Command, DeliveryMode, InboxQuery, SendMessage},
        ids::*,
        output::OutputSpec,
        pagination::PageRequest,
        results::{CommandResult, InboxBatchV2Item},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    };
    use crate::store::{connection::StoreContext, messages};
    use crate::test_support::isolation::TestIsolation;
    use rusqlite::Connection;
    use std::sync::Arc;

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(1000)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }

    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(1000),
            cancellation: Default::default(),
        }
    }

    fn add_member(conn: &Connection, id: &str) {
        let target = format!("p{id}");
        let execution = format!("00000000-0000-4000-8000-0000000000{:02x}", id.as_bytes()[0]);
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES (?1,'i','resolved','native',?2,1,1,0,1)",rusqlite::params![id,target]).unwrap();
        conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh',?2,'inc','coherent_enumeration',1)",rusqlite::params![target,format!("term-{target}")]).unwrap();
        conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,?2,'b',1,1,'codex',?1,?3,'cooperative_top_level',0,0,?4,'inc')",rusqlite::params![id,target,execution,format!("term-{target}")]).unwrap();
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [id],
        )
        .unwrap();
        conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [id]).unwrap();
    }

    fn setup(iso: &TestIsolation) -> (StoreContext, Connection) {
        let context = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock));
        let conn = context.open_writer().unwrap();
        conn.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1)", []).unwrap();
        conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)", []).unwrap();
        add_member(&conn, "a");
        conn.execute("UPDATE occupant_bindings SET execution_id='00000000-0000-4000-8000-0000000000aa',native_session='n' WHERE seat_id='a'",[]).unwrap();
        add_member(&conn, "b");
        (context, conn)
    }

    fn request(operation: &str, ack_b: bool) -> SendMessage {
        SendMessage {
            priority: None,
            delivery_mode: DeliveryMode::Ordinary,
            thread: ThreadId::new("t"),
            body: "hello".into(),
            direct_recipient: None,
            invited_recipients: if ack_b {
                vec![SeatId::new("b")]
            } else {
                vec![]
            },
            deadline_millis: None,
            operation: OperationId::new(operation),
            claim: CallerClaim {
                instance: "i".into(),
                seat: SeatId::new("a"),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new("n"),
                execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
                target: HostTargetId::new("pa"),
                agent_id: None,
                liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
            },
            relays_user: false,
            user_intent: None,
        }
    }

    fn permit(request: &SendMessage) -> MutationPermit {
        let digest =
            crate::store::schema::canonical_digest(&messages::send_payload(request)).unwrap();
        MutationPermit::cooperative(
            request.claim.clone(),
            request.operation.clone(),
            ObligationRef::Control(request.thread.clone()),
            digest,
            MonoInstant(50),
            (1, 0),
            CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default(),
            },
        )
    }

    fn prepare(context: &StoreContext, conn: &mut Connection, request: &SendMessage) {
        loop {
            let step = messages::prepare_send_step(
                context,
                conn,
                request,
                messages::MessageLimits::default(),
                &budget(),
                DurableWorkAdmission::new(16).unwrap(),
            )
            .unwrap();
            if matches!(step, SendPreparationProgress::Ready { .. }) {
                break;
            }
        }
    }

    fn send(context: &StoreContext, conn: &mut Connection, request: &SendMessage) -> MessageId {
        prepare(context, conn, request);
        match messages::publish_send(
            context,
            conn,
            request,
            &mut permit(request),
            &budget(),
            || messages::MAX_BODY_BYTES,
        )
        .unwrap()
        {
            CommandResult::MessageSent(id) => id,
            other => panic!("unexpected result {other:?}"),
        }
    }

    fn page(context: &StoreContext, claim: Option<CallerClaim>, pushed: bool) -> Vec<String> {
        let CommandResult::InboxBatchV2(page) = crate::store::queries::query_with_output(
            context,
            "i",
            &Command::InboxBatchV2(InboxQuery {
                pushed,
                thread: None,
                claim,
                seat: Some(SeatId::new("b")),
                page: PageRequest::default(),
            }),
            &OutputSpec::default(),
            &budget(),
        )
        .unwrap() else {
            panic!("v2 page")
        };
        assert!(!page.has_more, "pushed={pushed}");
        page.items
            .iter()
            .map(|item| match item {
                InboxBatchV2Item::Message { message, .. } => format!("ack {}", message.as_str()),
                InboxBatchV2Item::LazyMessage { message, .. } => {
                    format!("row {}", message.as_str())
                }
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn history_and_staged_backlog_do_not_hide_the_first_inbox_and_hook_page() {
        let iso = TestIsolation::new("deadline-window-history");
        let (context, mut conn) = setup(&iso);
        for n in 0..500 {
            let id = send(&context, &mut conn, &request(&format!("hist{n}"), true));
            conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES (?1,'b','acked','b',1,'o',1) ON CONFLICT(message_id,seat_id) DO UPDATE SET state='acked',ack_actor_seat_id='b',ack_generation=1,ack_observation='o',acked_at=1", [id.as_str()]).unwrap();
        }
        conn.execute("INSERT INTO binding_cursors(binding_id,thread_id,seq) SELECT binding_id,'t',(SELECT max(sequence) FROM messages) FROM occupant_bindings WHERE seat_id='b' ON CONFLICT(binding_id,thread_id) DO UPDATE SET seq=excluded.seq", []).unwrap();
        for n in 0..100 {
            prepare(&context, &mut conn, &request(&format!("staged{n}"), true));
        }
        let ack = send(&context, &mut conn, &request("p0ack", true));
        let row = send(&context, &mut conn, &request("p1", false));
        let want = vec![
            format!("ack {}", ack.as_str()),
            format!("row {}", row.as_str()),
        ];
        let mut claim = request("claim", false).claim;
        claim.seat = SeatId::new("b");
        claim.native_session = NativeSessionId::new("b");
        claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000062");
        claim.target = HostTargetId::new("pb");
        for pushed in [false, true] {
            assert_eq!(
                page(&context, Some(claim.clone()), pushed),
                want,
                "pushed={pushed}"
            );
            assert_eq!(
                page(&context, None, pushed),
                want,
                "seat-level pushed={pushed}"
            );
        }
    }
}
