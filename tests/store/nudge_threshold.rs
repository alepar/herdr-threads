//! Wake by per-seat nudge threshold (ht-yha.4): every wake decision for delivery rows goes through
//! `NudgeThreshold::admits` (Rust) or `THRESHOLD_ADMITS_SQL` (SQL), and rows arrive through the
//! real send path (`prepare_send_step` / `publish_send`).
use super::*;
use crate::ports::{DurableWorkAdmission, SendPreparationProgress};
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::{DeliveryMode, SendMessage},
    ids::*,
    priority::{DeliveryPriority, NudgeThreshold},
    results::CommandResult,
    time::{Cancellation, MonoInstant},
};
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
const DAEMON_BOOT: &str = "00000000-0000-0000-0000-000000000001";

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

/// Idle, registered recipient `s` (codex, threshold `threshold`), sender `a` and optionally the
/// extra agent `x`, all joined to thread `t`.
fn fixture(threshold: i64, extra: bool) -> (SqliteStore, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("herdr-nudge-thr-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute_batch(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
         INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);",
    )
    .unwrap();
    for (seat, execution, idle) in [
        ("s", "exec", true),
        ("a", "00000000-0000-4000-8000-0000000000aa", false),
        ("x", "00000000-0000-4000-8000-0000000000bb", false),
    ] {
        if seat == "x" && !extra {
            continue;
        }
        let target = format!("p{seat}");
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES (?1,'i','resolved','native',?2,1,1,0,1)", params![seat, target]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'host',1,1,0,'fresh',?2,'inc','coherent_enumeration',1)", params![target, format!("term-{target}")]).unwrap();
        if idle {
            db.execute("UPDATE observed_targets SET occupancy='occupied',ui_state='idle',verified_execution=?2,top_level_occupant=1 WHERE instance_id='i' AND target_id=?1", params![target, execution]).unwrap();
        }
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,1,?2,'host',1,'codex',?3,?4,'fresh',0,0,?5,'inc')", params![seat, target, if seat == "a" { "n" } else { seat }, execution, format!("term-{target}")]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    db.execute(
        "UPDATE seats SET nudge_threshold=?1 WHERE id='s'",
        [threshold],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
            wake_batch_delay_ms: 0,
            ..StoreSettings::default()
        },
    )
    .unwrap();
    (store, path)
}

fn request(priority: Option<u8>, operation: &str) -> SendMessage {
    SendMessage {
        priority: priority.map(|p| DeliveryPriority::new(p).unwrap()),
        delivery_mode: DeliveryMode::Ordinary,
        thread: ThreadId::new("t"),
        body: "hello".into(),
        direct_recipient: None,
        invited_recipients: vec![],
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

fn prepare(store: &SqliteStore, request: &SendMessage) {
    let mut conn = store.context.open_writer().unwrap();
    loop {
        let step = messages::prepare_send_step(
            &store.context,
            &mut conn,
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

fn publish(store: &SqliteStore, request: &SendMessage) -> MessageId {
    let mut conn = store.context.open_writer().unwrap();
    let digest = schema::canonical_digest(&messages::send_payload(request)).unwrap();
    let mut permit = MutationPermit::cooperative(
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
    );
    match messages::publish_send(
        &store.context,
        &mut conn,
        request,
        &mut permit,
        &budget(),
        || messages::MAX_BODY_BYTES,
    )
    .unwrap()
    {
        CommandResult::MessageSent(id) => id,
        other => panic!("unexpected result {other:?}"),
    }
}

/// A real send from `a` at `priority` (`None`: the legacy ordinary-mode default).
fn send(store: &SqliteStore, priority: Option<u8>, operation: &str) -> MessageId {
    let request = request(priority, operation);
    prepare(store, &request);
    publish(store, &request)
}

fn set_threshold(store: &SqliteStore, value: i64) {
    let db = store.context.open_writer().unwrap();
    db.execute("UPDATE seats SET nudge_threshold=?1 WHERE id='s'", [value])
        .unwrap();
    // What the accountable setter does after a change.
    let tx = db.unchecked_transaction().unwrap();
    wake::note_new_attention(&tx, &SeatId::new("s")).unwrap();
    tx.commit().unwrap();
}

fn threshold_of(value: i64) -> NudgeThreshold {
    NudgeThreshold::new(value).unwrap()
}

fn frontier(store: &SqliteStore) -> Option<i64> {
    let db = store.context.open_writer().unwrap();
    attention::wake_seat_attention(&db, "s")
        .unwrap()
        .attention
        .frontier
        .lazy
}

fn has_wake_rows(store: &SqliteStore) -> bool {
    let db = store.context.open_writer().unwrap();
    lazy_delivery::has_wake_rows(&db, "s").unwrap()
}

fn candidates(store: &SqliteStore) -> Vec<WakeCandidate> {
    StorePort::wake_candidates(store, PageRequest::default(), &budget())
        .unwrap()
        .items
}

fn direct_candidate(store: &SqliteStore) -> WakeCandidate {
    let db = store.context.open_writer().unwrap();
    let scan = attention::wake_seat_attention(&db, "s").unwrap();
    wake::load_candidate(
        &db,
        "i",
        &SeatId::new("s"),
        &scan.attention,
        scan.decision_seq,
    )
    .unwrap()
}

fn reserve(store: &SqliteStore, candidate: &WakeCandidate) -> Option<WakeReservation> {
    StorePort::reserve_wake(store, candidate, &budget()).unwrap()
}

fn settle(store: &SqliteStore, reservation: &WakeReservation) {
    StorePort::complete_wake(
        store,
        reservation.attempt.clone(),
        WakeOutcome::Submitted,
        None,
        &budget(),
    )
    .unwrap();
}

fn last_lazy_ordinal(store: &SqliteStore) -> Option<i64> {
    store
        .context
        .open_writer()
        .unwrap()
        .query_row(
            "SELECT last_lazy_ordinal FROM wake_work WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
        .flatten()
}

/// The publication decision sequence of the newest delivery row of `s`.
fn newest_row_decision(store: &SqliteStore) -> i64 {
    store
        .context
        .open_writer()
        .unwrap()
        .query_row(
            "SELECT max(decision_seq) FROM lazy_recipients WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn cleanup(store: SqliteStore, path: std::path::PathBuf) {
    drop(store);
    let _ = std::fs::remove_file(path);
}

// Kills: a wake decision that compares priority against T by hand with the wrong bound, ignores a
// negative T, or wakes on a row the threshold does not admit.
#[test]
fn threshold_matrix_wakes_iff_admitted() {
    for threshold in [-1, 0, 3] {
        for priority in [0u8, 1, 5] {
            let (store, path) = fixture(threshold, false);
            send(&store, Some(priority), "op");
            let admitted = threshold_of(threshold).admits(DeliveryPriority::new(priority).unwrap());
            assert_eq!(
                frontier(&store).is_some(),
                admitted,
                "T={threshold} p{priority}: frontier"
            );
            assert_eq!(
                has_wake_rows(&store),
                admitted,
                "T={threshold} p{priority}: probe"
            );
            assert_eq!(
                candidates(&store).len(),
                usize::from(admitted),
                "T={threshold} p{priority}: candidate"
            );
            cleanup(store, path);
        }
    }
}

// Kills: a SQL twin of `admits` that drifts from the Rust predicate.
#[test]
fn sql_fragment_matches_admits() {
    let (store, path) = fixture(0, false);
    let db = store.context.open_writer().unwrap();
    let sql = format!(
        "SELECT {} FROM (SELECT ?1 AS priority) r, (SELECT ?2 AS nudge_threshold) s",
        seats::THRESHOLD_ADMITS_SQL
    );
    for threshold in -1..=9i64 {
        for priority in 0..=9i64 {
            let in_sql: bool = db
                .query_row(&sql, params![priority, threshold], |r| r.get(0))
                .unwrap();
            let admits =
                threshold_of(threshold).admits(DeliveryPriority::new(priority as u8).unwrap());
            assert_eq!(in_sql, admits, "T={threshold} p{priority}");
        }
    }
    drop(db);
    cleanup(store, path);
}

// Kills: raising T that wakes once per row instead of once per frontier, or never wakes for rows
// that were already pending.
#[test]
fn raising_threshold_wakes_once_for_pending_rows() {
    let (store, path) = fixture(0, false);
    send(&store, Some(3), "op1");
    send(&store, Some(3), "op2");
    assert!(candidates(&store).is_empty(), "T=0 does not admit p3");
    set_threshold(&store, 3);
    let found = candidates(&store);
    assert_eq!(found.len(), 1, "one candidate for both rows");
    let reservation = reserve(&store, &found[0]).expect("the raised seat wakes");
    assert_eq!(reservation.reasons, vec!["lazy".to_string()]);
    assert_eq!(last_lazy_ordinal(&store), Some(newest_row_decision(&store)));
    settle(&store, &reservation);
    assert!(
        candidates(&store).is_empty(),
        "no second wake for the same frontier"
    );
    assert!(!direct_candidate(&store).has_actionable_work());
    assert!(reserve(&store, &direct_candidate(&store)).is_none());
    cleanup(store, path);
}

// Kills: a lowered threshold that still wakes for rows it no longer admits.
#[test]
fn lowering_threshold_stops_new_wakes() {
    let (store, path) = fixture(3, false);
    send(&store, Some(3), "op1");
    let first = candidates(&store).remove(0);
    let reservation = reserve(&store, &first).expect("T=3 wakes for p3");
    settle(&store, &reservation);
    set_threshold(&store, 0);
    send(&store, Some(3), "op2");
    assert!(candidates(&store).is_empty());
    assert_eq!(frontier(&store), None);
    assert!(reserve(&store, &direct_candidate(&store)).is_none());
    cleanup(store, path);
}

// Kills: a default seat that is not woken by an urgent row, or woken twice for it.
#[test]
fn p0_row_wakes_default_seat_once() {
    let (store, path) = fixture(0, false);
    send(&store, Some(0), "op");
    let reservation = reserve(&store, &candidates(&store).remove(0)).expect("p0 wakes T=0");
    assert_eq!(last_lazy_ordinal(&store), Some(newest_row_decision(&store)));
    settle(&store, &reservation);
    assert!(candidates(&store).is_empty());
    assert!(reserve(&store, &direct_candidate(&store)).is_none());
    cleanup(store, path);
}

/// `n` published priority-1 delivery rows to `s` (bulk fixture rows: preparation, recipient,
/// message, manifest, then the real `note_published`), after sequence and decision floors `base`.
fn bulk_p1_rows(store: &SqliteStore, n: i64) {
    let mut db = store.context.open_writer().unwrap();
    let tx = db.transaction().unwrap();
    for k in 1..=n {
        let seq = 100 + k;
        tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES(?1,'i','scope',?1,zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy',1)", [format!("prep-{k}")]).unwrap();
        tx.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES(?1,?2,'t','s',1)", params![format!("prep-{k}"), format!("msg-{k}")]).unwrap();
        tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES(?1,'i','t',?2,'ordinary',?2,'passive',?2,'lazy',1)", params![format!("msg-{k}"), seq]).unwrap();
        tx.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES(?1,?2,'i','t',?3,?3,?3,0,0,0)", params![format!("prep-{k}"), format!("msg-{k}"), seq]).unwrap();
        lazy_delivery::note_published(&tx, &format!("prep-{k}")).unwrap();
    }
    tx.execute(
        "UPDATE threads SET next_sequence=?1 WHERE id='t'",
        [100 + n + 1],
    )
    .unwrap();
    tx.execute(
        "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
        [100 + n + 1],
    )
    .unwrap();
    tx.commit().unwrap();
}

// Kills: a frontier or probe that reads a bounded newest-N window of all pending rows (the old
// PENDING_SUMMARY_CAP) or scans past admitted priorities, hiding an urgent row behind a backlog.
#[test]
fn p0_row_behind_1001_p1_rows_wakes() {
    let (store, path) = fixture(0, false);
    bulk_p1_rows(&store, 1001);
    assert!(
        !has_wake_rows(&store) && frontier(&store).is_none(),
        "T=0 does not admit the p1 backlog"
    );
    send(&store, Some(0), "urgent");
    assert_eq!(frontier(&store), Some(newest_row_decision(&store)));
    assert!(has_wake_rows(&store));
    assert_eq!(candidates(&store).len(), 1);
    let db = store.context.open_writer().unwrap();
    let urgent: i64 = db
        .query_row(
            "SELECT decision_seq FROM lazy_recipients WHERE seat_id='s' AND priority=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(frontier(&store), Some(urgent));
    let plan: Vec<String> = db
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            lazy_delivery::FRONTIER_SEEK_SQL
        ))
        .unwrap()
        .query_map(params!["any-binding", 0], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        plan.iter()
            .any(|step| step.contains("binding_pending_order") && step.contains("SEARCH")),
        "{plan:?}"
    );
    drop(db);
    cleanup(store, path);
}

// Kills: a staged (unpublished) row that wakes the seat or feeds the probe.
#[test]
fn staged_rows_never_count() {
    let (store, path) = fixture(-1, false);
    prepare(&store, &request(Some(0), "staged"));
    let rows: i64 = store
        .context
        .open_writer()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='s' AND decision_seq IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1, "the row is staged");
    assert!(!has_wake_rows(&store));
    assert_eq!(frontier(&store), None);
    assert!(candidates(&store).is_empty());
    cleanup(store, path);
}

// Kills: a first-contact enrolment (cursor at the thread head) that still leaves a row that wakes.
#[test]
fn first_contact_fast_forward_leaves_nothing_that_wakes() {
    let (store, path) = fixture(-1, false);
    send(&store, Some(0), "op1");
    send(&store, Some(1), "op2");
    assert!(has_wake_rows(&store), "before: pending rows wake the seat");
    let db = store.context.open_writer().unwrap();
    let binding: String = db
        .query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // A binding enrolled at the thread head: the seed of a first contact.
    crate::store::read_cursors::init_for_new_binding(&db, &BindingId::new(&binding)).unwrap();
    drop(db);
    assert!(!has_wake_rows(&store));
    assert_eq!(frontier(&store), None);
    assert!(candidates(&store).is_empty());
    cleanup(store, path);
}

// Kills: a subagent's display that marks the seat-wide row displayed and so drops a p0 row the
// top-level binding has not seen from the frontier.
#[test]
fn subagent_display_keeps_p0_row_in_frontier() {
    let (store, path) = fixture(0, false);
    let id = send(&store, Some(0), "op");
    let mut db = store.context.open_writer().unwrap();
    let parent: String = db
        .query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,role,parent_binding,agent_id) VALUES ('s',0,'ps','host',1,0,'codex','s',?1,'cooperative_top_level',0,0,'term-ps','inc','subagent',?2,'ag1')", params![uuid::Uuid::new_v4().to_string(), parent]).unwrap();
    let sub: String = db
        .query_row(
            "SELECT binding_id FROM occupant_bindings WHERE agent_id='ag1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let before = newest_row_decision(&store);
    let tx = db.transaction().unwrap();
    lazy_delivery::complete_for_binding(
        &tx,
        "i",
        &SeatId::new("s"),
        Some(&BindingId::new(&sub)),
        std::slice::from_ref(&id),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(db);
    assert_eq!(
        frontier(&store),
        Some(before),
        "the p0 row is still pending"
    );
    assert!(has_wake_rows(&store));
    // The top-level binding's own display does clear it.
    let mut db = store.context.open_writer().unwrap();
    let tx = db.transaction().unwrap();
    lazy_delivery::complete_for_binding(
        &tx,
        "i",
        &SeatId::new("s"),
        Some(&BindingId::new(&parent)),
        std::slice::from_ref(&id),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(db);
    assert_eq!(frontier(&store), None);
    assert!(!has_wake_rows(&store));
    cleanup(store, path);
}

// Kills: a mixed ACK send whose non-explicit member gets no row that wakes it at the default
// threshold (the p0 class of its row).
#[test]
fn mixed_ack_send_non_explicit_member_row_wakes_it() {
    let (store, path) = fixture(0, true);
    let mut request = request(None, "ack");
    request.invited_recipients = vec![SeatId::new("x")];
    prepare(&store, &request);
    publish(&store, &request);
    let db = store.context.open_writer().unwrap();
    let row: (i64, i64) = db
        .query_row(
            "SELECT priority,decision_seq IS NOT NULL FROM lazy_recipients WHERE seat_id='s'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, (0, 1));
    drop(db);
    assert!(has_wake_rows(&store));
    let found = candidates(&store);
    assert!(
        found.iter().any(|c| c.seat.as_str() == "s"),
        "the non-explicit member is a wake candidate"
    );
    cleanup(store, path);
}

// Kills: per-item `wakes` frozen at send time instead of read against the current threshold.
#[test]
fn hydrate_wakes_follow_current_threshold() {
    use crate::protocol::{
        commands::{Command, InboxQuery},
        output::OutputSpec,
        results::InboxBatchV2Item,
    };
    let (store, path) = fixture(0, false);
    send(&store, Some(1), "op");
    let wakes = |store: &SqliteStore| -> Vec<bool> {
        let query = InboxQuery {
            pushed: false,
            seat: Some(SeatId::new("s")),
            page: PageRequest::default(),
            claim: None,
            thread: None,
        };
        let result = crate::store::queries::query_with_output(
            &store.context,
            "i",
            &Command::InboxBatchV2(query),
            &OutputSpec::default(),
            &budget(),
        )
        .unwrap();
        let CommandResult::InboxBatchV2(page) = result else {
            panic!("unexpected result");
        };
        page.items
            .into_iter()
            .filter_map(|item| match item {
                InboxBatchV2Item::LazyMessage { wakes, .. } => Some(wakes),
                _ => None,
            })
            .collect()
    };
    assert_eq!(wakes(&store), vec![false], "T=0 does not admit p1");
    set_threshold(&store, 1);
    assert_eq!(wakes(&store), vec![true], "T=1 admits it, with no re-send");
    set_threshold(&store, 0);
    assert_eq!(wakes(&store), vec![false]);
    cleanup(store, path);
}
