//! First-contact history stats and resume fast-forward (ht-54r.8, spec §2).
use super::*;
use crate::ports::StorePort;
use crate::protocol::{
    authority::{CallerClaim, LivenessFingerprint},
    commands::{Command, InboxQuery},
    ids::{ExecutionId, OperationId},
    output::OutputSpec,
    pagination::PageRequest,
    results::{CommandResult, InboxBatchV2Item},
    summary::{ItemBody, OpenItemKind},
};
use crate::store::{
    bindings::{self, DisconnectReason},
    connection::StoreContext,
    cooperative_checkin_tests::{budget, check_in, fixture, lifecycle, session_claim},
    queries::query_with_output,
};
use crate::test_support::isolation::TestIsolation;
use std::sync::atomic::{AtomicI64, Ordering};

const MINUTE: i64 = 60_000;

static DECISION: AtomicI64 = AtomicI64::new(1000);

fn next_decision() -> i64 {
    DECISION.fetch_add(1, Ordering::SeqCst)
}

fn seat(conn: &Connection, id: &str) {
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES (?1,'i','resolved','native',?2,1,1,0,1)", params![id, format!("p{id}")]).unwrap();
    conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh',?2,'inc','coherent_enumeration',1)", params![format!("p{id}"), format!("term-p{id}")]).unwrap();
}

/// A thread (with an optional name) that `members` have joined.
fn thread(conn: &Connection, id: &str, name: Option<&str>, members: &[&str]) {
    conn.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,name) VALUES (?1,'i','topic','goal',0,0,?2)",
        params![id, name],
    )
    .unwrap();
    for member in members {
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,?2,'joined')",
            params![id, member],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,1,1)",
            params![id, member],
        )
        .unwrap();
    }
}

/// How a raw message was authored.
#[derive(Clone, Copy)]
enum By {
    Human,
    Agent,
    Relay,
}

fn message(conn: &Connection, thread: &str, sequence: i64, by: By, at: i64, body: &str) -> String {
    let id = format!("{thread}-m{sequence}");
    let (role, relays) = match by {
        By::Human => ("human", 0),
        By::Agent => ("agent", 0),
        By::Relay => ("agent", 1),
    };
    conn.execute(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq,author_role,relays_user) VALUES (?1,'i',?2,?3,'ordinary','o',?4,?5,?6,?7,?8)",
        params![id, thread, sequence, body, at, next_decision(), role, relays],
    )
    .unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=MAX(decision_seq,(SELECT MAX(decision_seq) FROM messages))",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE threads SET next_sequence=?2 WHERE id=?1 AND next_sequence<=?3",
        params![thread, sequence + 1, sequence],
    )
    .unwrap();
    id
}

/// A message addressed to `recipient` through the send manifest (what the inbox and the pending
/// ACK obligations are built from).
fn deliver(conn: &Connection, thread: &str, sequence: i64, recipient: &str, body: &str) -> String {
    let id = message(conn, thread, sequence, By::Agent, sequence * 10, body);
    conn.execute_batch(&format!(
        "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p-{id}','i','actor','{id}',zeroblob(32),'{thread}',0,0,0,0,0,0,1,'sealed');
         INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p-{id}','{thread}','{recipient}',1,300,1);
         INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) SELECT 'p-{id}','{id}','i','{thread}',decision_seq,decision_at,{sequence},0,1,0 FROM messages WHERE id='{id}';"
    ))
    .unwrap();
    id
}

/// A level-0 ledger block over `thread` holding `items` (seq, id, body) and transitions
/// `(target, status, cite_seq)`.
fn ledger(
    conn: &Connection,
    thread: &str,
    items: &[(i64, &str, ItemBody)],
    transitions: &[(&str, &str, i64)],
) {
    let block = format!("block-{thread}");
    conn.execute(
        "INSERT INTO summary_blocks(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,narrative,author_seat_id,model,prompt_version,created_at) VALUES (?1,'i',?2,'v1',0,0,1,100,'h','n','o','m','p',5)",
        params![block, thread],
    )
    .unwrap();
    for (seq, id, body) in items {
        let kind = match body {
            ItemBody::UserInstruction { .. } => "user_instruction",
            ItemBody::Decision { .. } => "decision",
            ItemBody::OpenItem { .. } => "open_item",
        };
        conn.execute(
            "INSERT INTO summary_items(block_id,kind,item_id,seq,body_json,thread_id,chunking_version) VALUES (?1,?2,?3,?4,?5,?6,'v1')",
            params![block, kind, id, seq, serde_json::to_string(body).unwrap(), thread],
        )
        .unwrap();
    }
    for (ordinal, (target, status, cite)) in transitions.iter().enumerate() {
        conn.execute(
            "INSERT INTO summary_transitions(block_id,ordinal,thread_id,chunking_version,target_id,new_status,cite_seq) VALUES (?1,?2,?3,'v1',?4,?5,?6)",
            params![block, ordinal as i64, thread, target, status, cite],
        )
        .unwrap();
    }
}

fn open_item(kind: OpenItemKind) -> ItemBody {
    ItemBody::OpenItem {
        kind,
        from_seat: SeatId::new("o"),
        to_seat: None,
        text: "please".into(),
    }
}

struct Db {
    _iso: TestIsolation,
    conn: Connection,
}

/// Seats `s` (the reader) and `o` (the author) and nothing else.
fn db(name: &str) -> Db {
    struct Clock;
    impl crate::protocol::time::Clock for Clock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(1000)
        }
        fn monotonic_now(&self) -> crate::protocol::time::MonoInstant {
            crate::protocol::time::MonoInstant(100)
        }
    }
    let iso = TestIsolation::new(name);
    let conn = StoreContext::new(iso.path("store.db"), std::sync::Arc::new(Clock))
        .open_writer()
        .unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1)",
        [],
    )
    .unwrap();
    seat(&conn, "s");
    seat(&conn, "o");
    Db { _iso: iso, conn }
}

fn stats(conn: &Connection, cursors: &[(&str, u64)]) -> FirstContact {
    let cursors: std::collections::HashMap<String, u64> = cursors
        .iter()
        .map(|(thread, seq)| ((*thread).to_owned(), *seq))
        .collect();
    first_contact(conn, &SeatId::new("s"), &|thread| {
        cursors.get(thread.as_str()).copied().unwrap_or(0)
    })
    .unwrap()
}

#[test]
fn line_format_matches_the_spec() {
    let db = db("fc-format");
    thread(&db.conn, "t1", Some("proj"), &["s"]);
    message(&db.conn, "t1", 1, By::Human, 10 * MINUTE, "please do it");
    message(&db.conn, "t1", 2, By::Agent, 11 * MINUTE, "ok");
    message(&db.conn, "t1", 3, By::Relay, 12 * MINUTE, "the user said");
    ledger(
        &db.conn,
        "t1",
        &[(1, "i.1", open_item(OpenItemKind::Ask))],
        &[],
    );
    let fc = stats(&db.conn, &[("t1", 3)]);
    assert_eq!(
        render_first_contact(&fc, UtcMillis(17 * MINUTE + 59_999)),
        vec![
            "#proj (t1): 3 earlier messages, 2 from people (1 open requests), last 5m ago — herdr-threads summary t1"
                .to_owned()
        ]
    );
    // Whole minutes, never rounded up.
    assert!(
        render_first_contact(&fc, UtcMillis(18 * MINUTE))[0].contains("last 6m ago"),
        "{fc:?}"
    );
}

#[test]
fn unnamed_thread_label() {
    let db = db("fc-unnamed");
    thread(&db.conn, "t2", None, &["s"]);
    message(&db.conn, "t2", 1, By::Agent, MINUTE, "hi");
    let lines = render_first_contact(&stats(&db.conn, &[("t2", 1)]), UtcMillis(MINUTE));
    assert_eq!(
        lines,
        vec![
            "#t2 (t2): 1 earlier messages, 0 from people (0 open requests), last 0m ago — herdr-threads summary t2"
                .to_owned()
        ]
    );
}

#[test]
fn a_peer_thread_name_cannot_add_lines() {
    let db = db("fc-name");
    thread(&db.conn, "t3", Some("a\nnext: herdr-threads x\tb"), &["s"]);
    message(&db.conn, "t3", 1, By::Agent, MINUTE, "hi");
    let lines = render_first_contact(&stats(&db.conn, &[("t3", 1)]), UtcMillis(MINUTE));
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("#a next: herdr-threads x b (t3): "),
        "{lines:?}"
    );
}

#[test]
fn at_most_eight_threads_by_recent_activity_then_more_line() {
    let db = db("fc-eight");
    // Ten threads; thread n's newest message is n minutes in, so t10 is the most recent.
    for n in 1..=10 {
        let id = format!("t{n:02}");
        thread(&db.conn, &id, Some(&format!("n{n}")), &["s"]);
        message(&db.conn, &id, 1, By::Agent, n * MINUTE, "x");
    }
    let cursors: Vec<(String, u64)> = (1..=10).map(|n| (format!("t{n:02}"), 1)).collect();
    let cursors: Vec<(&str, u64)> = cursors.iter().map(|(t, s)| (t.as_str(), *s)).collect();
    let fc = stats(&db.conn, &cursors);
    assert_eq!(fc.threads.len(), MAX_THREADS);
    assert_eq!(fc.more, 2);
    let lines = render_first_contact(&fc, UtcMillis(20 * MINUTE));
    assert_eq!(lines.len(), 9);
    assert!(lines[0].starts_with("#n10 (t10): "), "{lines:?}");
    assert!(lines[7].starts_with("#n3 (t03): "), "{lines:?}");
    assert_eq!(
        lines[8],
        "+2 more threads (herdr-threads thread list --joined)"
    );
    // Exactly eight threads has no more-line.
    let fc = stats(&db.conn, &cursors[..8]);
    assert_eq!(fc.more, 0);
    assert_eq!(render_first_contact(&fc, UtcMillis(20 * MINUTE)).len(), 8);
}

#[test]
fn threads_without_earlier_history_are_omitted() {
    let db = db("fc-omitted");
    thread(&db.conn, "empty", None, &["s"]);
    thread(&db.conn, "at-zero", None, &["s"]);
    message(&db.conn, "at-zero", 1, By::Agent, MINUTE, "x");
    thread(&db.conn, "not-joined", None, &["o"]);
    message(&db.conn, "not-joined", 1, By::Agent, MINUTE, "x");
    thread(&db.conn, "has", None, &["s"]);
    message(&db.conn, "has", 1, By::Agent, MINUTE, "x");
    let fc = stats(
        &db.conn,
        &[("empty", 0), ("at-zero", 0), ("not-joined", 1), ("has", 1)],
    );
    assert_eq!(
        fc.threads
            .iter()
            .map(|t| t.thread.as_str())
            .collect::<Vec<_>>(),
        vec!["has"]
    );
    assert_eq!(fc.more, 0);
    // A seat with no history anywhere gets no stats at all.
    assert!(stats(&db.conn, &[]).threads.is_empty());
}

#[test]
fn counts_cover_only_messages_up_to_the_cursor() {
    let db = db("fc-cursor");
    thread(&db.conn, "t", Some("t"), &["s"]);
    message(&db.conn, "t", 1, By::Agent, MINUTE, "a");
    message(&db.conn, "t", 2, By::Human, 2 * MINUTE, "b");
    message(&db.conn, "t", 3, By::Agent, 3 * MINUTE, "c");
    message(&db.conn, "t", 4, By::Agent, 4 * MINUTE, "d");
    message(&db.conn, "t", 5, By::Relay, 5 * MINUTE, "e");
    message(&db.conn, "t", 6, By::Agent, 6 * MINUTE, "f");
    let at3 = stats(&db.conn, &[("t", 3)]).threads[0].clone();
    assert_eq!(
        (at3.earlier, at3.from_people, at3.last_at),
        (3, 1, UtcMillis(3 * MINUTE))
    );
    let at5 = stats(&db.conn, &[("t", 5)]).threads[0].clone();
    assert_eq!(
        (at5.earlier, at5.from_people, at5.last_at),
        (5, 2, UtcMillis(5 * MINUTE))
    );
}

#[test]
fn open_requests_come_from_the_summary_ledger() {
    let db = db("fc-ledger");
    thread(&db.conn, "t", Some("t"), &["s"]);
    for n in 1..=8 {
        message(&db.conn, "t", n, By::Agent, n * MINUTE, "x");
    }
    // An open ask and an open question count; a commitment and a blocker do not; an ask
    // resolved by a later transition does not; an item past the cursor does not.
    ledger(
        &db.conn,
        "t",
        &[
            (1, "ask", open_item(OpenItemKind::Ask)),
            (2, "question", open_item(OpenItemKind::Question)),
            (3, "promise", open_item(OpenItemKind::Commitment)),
            (4, "stuck", open_item(OpenItemKind::Blocker)),
            (5, "answered", open_item(OpenItemKind::Ask)),
            (7, "later", open_item(OpenItemKind::Ask)),
        ],
        &[("answered", "resolved", 6)],
    );
    assert_eq!(stats(&db.conn, &[("t", 6)]).threads[0].open_requests, 2);
    // Before the resolving message the answered ask is still open; past the later item it counts.
    assert_eq!(stats(&db.conn, &[("t", 5)]).threads[0].open_requests, 3);
    assert_eq!(stats(&db.conn, &[("t", 8)]).threads[0].open_requests, 3);
    // A thread nothing summarized has none.
    thread(&db.conn, "plain", None, &["s"]);
    message(&db.conn, "plain", 1, By::Agent, MINUTE, "x");
    assert_eq!(stats(&db.conn, &[("plain", 1)]).threads[0].open_requests, 0);
}

// ---- check-in level ----

const EXEC_A: &str = "00000000-0000-4000-8000-0000000000a1";
const EXEC_B: &str = "00000000-0000-4000-8000-0000000000b1";

/// The fixture seat `s` with an author seat `o` and a joined thread `t` of `n` messages from `o`.
fn seated(
    n: i64,
) -> (
    crate::store::cooperative_checkin_tests::OwnedFixture<crate::store::SqliteStore>,
    crate::store::cooperative_checkin_tests::OwnedFixture<Connection>,
) {
    let (store, conn, _) = fixture();
    seat(&conn, "o");
    thread(&conn, "t", Some("proj"), &["s"]);
    for seq in 1..=n {
        message(&conn, "t", seq, By::Human, seq * MINUTE, "history");
    }
    (store, conn)
}

fn binding_id(conn: &Connection, session: &str) -> BindingId {
    BindingId::new(
        conn.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE native_session=?1 AND agent_id=''",
            [session],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
}

fn cursor(conn: &Connection, session: &str) -> Option<u64> {
    crate::store::read_cursors::cursor_for(conn, &binding_id(conn, session), &ThreadId::new("t"))
        .unwrap()
}

#[test]
fn first_contact_is_attached_once_per_binding() {
    let (store, _conn) = seated(5);
    let first = check_in(&store, lifecycle(session_claim("sess-A", EXEC_A), "a1")).unwrap();
    let fc = first.first_contact.expect("a new binding's first check-in");
    assert_eq!(fc.threads.len(), 1);
    assert_eq!(
        (
            fc.threads[0].thread.as_str(),
            fc.threads[0].earlier,
            fc.threads[0].from_people
        ),
        ("t", 5, 5)
    );
    assert!(
        first.fast_forward.is_none(),
        "enrollment never fast-forwards"
    );
    // The same session again (a second lifecycle check-in, a plain current one): not again.
    let mut same = first.context.clone();
    same.execution = ExecutionId::new(uuid::Uuid::new_v4().to_string());
    let again = check_in(&store, lifecycle(same, "a2")).unwrap();
    assert!(again.first_contact.is_none() && again.fast_forward.is_none());
    let current = crate::protocol::commands::CheckIn {
        lineage_parent: None,
        mode: crate::protocol::commands::CheckInMode::Current,
        claim: first.context.clone(),
        operation: OperationId::new("a3"),
    };
    let current = check_in(&store, current).unwrap();
    assert!(current.first_contact.is_none());
    // A second session is a new binding with its own first contact.
    let second = check_in(&store, lifecycle(session_claim("sess-B", EXEC_B), "b1")).unwrap();
    assert_eq!(second.first_contact.expect("its own").threads[0].earlier, 5);
}

#[test]
fn a_seat_with_no_history_gets_no_first_contact() {
    let (store, _conn) = seated(0);
    let first = check_in(&store, lifecycle(session_claim("sess-A", EXEC_A), "a1")).unwrap();
    assert!(first.first_contact.is_none());
}

/// Seat `s` has `sess-A` enrolled at the thread head (1), then a backlog arrives.
fn resumed(
    body_bytes: usize,
    backlog: i64,
) -> (
    crate::store::cooperative_checkin_tests::OwnedFixture<crate::store::SqliteStore>,
    crate::store::cooperative_checkin_tests::OwnedFixture<Connection>,
    crate::protocol::results::CheckInResult,
    Vec<String>,
) {
    let (store, conn) = seated(1);
    let first = check_in(&store, lifecycle(session_claim("sess-A", EXEC_A), "a1")).unwrap();
    assert_eq!(cursor(&conn, "sess-A"), Some(1));
    let mut ids = Vec::new();
    for seq in 2..2 + backlog {
        ids.push(deliver(&conn, "t", seq, "s", &"x".repeat(body_bytes)));
    }
    bindings::disconnect(
        &conn,
        &binding_id(&conn, "sess-A"),
        DisconnectReason::ProcessExit,
        UtcMillis(500),
    )
    .unwrap();
    (store, conn, first, ids)
}

fn resume(
    store: &crate::store::SqliteStore,
    first: &crate::protocol::results::CheckInResult,
    op: &str,
) -> crate::protocol::results::CheckInResult {
    // A lifecycle check-in always brings a fresh execution UUID; the session keys the binding.
    let mut claim = first.context.clone();
    claim.execution = ExecutionId::new(uuid::Uuid::new_v4().to_string());
    check_in(store, lifecycle(claim, op)).unwrap()
}

#[test]
fn fast_forward_triggers_above_one_page_and_not_below() {
    let page = ORDINARY_PAGE_BYTES as usize;
    // One message whose body plus the row overhead is exactly one page: it fits, no jump.
    let (store, conn, first, _) = resumed(page - ROW_OVERHEAD_BYTES as usize, 1);
    let back = resume(&store, &first, "r1");
    assert!(back.fast_forward.is_none() && back.first_contact.is_none());
    assert_eq!(cursor(&conn, "sess-A"), Some(1), "nothing moved");
    drop((store, conn));
    // One byte more: the backlog exceeds a page, the cursor jumps to the head.
    let (store, conn, first, _) = resumed(page - ROW_OVERHEAD_BYTES as usize + 1, 1);
    let back = resume(&store, &first, "r1");
    let ff = back.fast_forward.expect("over one page");
    assert!(back.first_contact.is_none());
    assert_eq!(ff.skipped, 1);
    assert_eq!(ff.threads.len(), 1);
    let head = crate::store::read_cursors::thread_head(&conn, &ThreadId::new("t")).unwrap();
    assert_eq!(cursor(&conn, "sess-A"), Some(head));
    let thread = &ff.threads[0];
    assert_eq!(thread.history.earlier, 1);
    assert_eq!(thread.history.from_people, 0);
    let last = thread.last_message.as_ref().expect("last message");
    assert_eq!(last.sequence, 2);
    assert!(last.preview_omitted);
    let lines = render_fast_forward(&ff, UtcMillis(30 * MINUTE));
    assert_eq!(lines[0], "resumed past 1 messages:");
    assert!(
        lines[1]
            .starts_with("#proj (t): 1 earlier messages, 0 from people (0 open requests), last "),
        "{lines:?}"
    );
    assert!(lines[1].ends_with("— herdr-threads summary t"), "{lines:?}");
    assert!(lines[2].starts_with("  last: #2 o: xxx"), "{lines:?}");
}

#[test]
fn many_small_messages_past_a_page_fast_forward_with_gap_counts() {
    // 70 messages of 200 bytes: 70 * (200 + 256) bytes is past one page.
    let (store, conn, first, _) = resumed(200, 70);
    let back = resume(&store, &first, "r1");
    let ff = back.fast_forward.expect("over one page");
    assert_eq!(ff.skipped, 70);
    assert_eq!(ff.threads[0].history.earlier, 70);
    let head = crate::store::read_cursors::thread_head(&conn, &ThreadId::new("t")).unwrap();
    assert_eq!(cursor(&conn, "sess-A"), Some(head));
    // Below a page nothing happens: 10 small messages.
    let (store, _conn, first, _) = resumed(200, 10);
    assert!(resume(&store, &first, "r1").fast_forward.is_none());
}

fn page_of(store: &crate::store::SqliteStore, claim: &CallerClaim) -> Vec<(String, bool)> {
    let CommandResult::InboxBatchV2(page) = query_with_output(
        &store.context,
        "i",
        &Command::InboxBatchV2(InboxQuery {
            claim: Some(claim.clone()),
            seat: Some(claim.seat.clone()),
            page: PageRequest {
                limit: 100,
                max_bytes: 60_000,
                cursor: None,
            },
        }),
        &OutputSpec::default(),
        &budget(),
    )
    .unwrap() else {
        panic!("v2 page")
    };
    page.items
        .into_iter()
        .filter_map(|item| match item {
            InboxBatchV2Item::Message {
                message,
                ack_candidate,
                ..
            } => Some((message.as_str().to_owned(), ack_candidate.is_some())),
            _ => None,
        })
        .collect()
}

#[test]
fn fast_forward_keeps_pending_obligations_listed() {
    // 25 messages of 800 bytes: past one page, and within what one inbox read walks.
    let (store, conn, first, ids) = resumed(800, 25);
    // The fixture seat starts at generation 0, which a v2 cursor refuses; real seats begin at 1.
    conn.execute("UPDATE occupant_bindings SET generation=1", [])
        .unwrap();
    conn.execute("UPDATE seats SET generation=1 WHERE id='s'", [])
        .unwrap();
    let mut first = first;
    first.context.binding_generation = 1;
    assert_eq!(
        page_of(&store, &first.context).len(),
        ids.len(),
        "before: the backlog is unread"
    );
    let back = resume(&store, &first, "r1");
    assert!(back.fast_forward.is_some());
    assert!(
        !crate::store::read_cursors::has_unread(&conn, &binding_id(&conn, "sess-A")).unwrap(),
        "the cursor is at the head: no unread mail"
    );
    // Every skipped message is still a pending ACK obligation of the seat and is still listed,
    // once, with the ACK offered to this top-level binding.
    let listed = page_of(&store, &back.context);
    assert_eq!(
        listed,
        ids.iter().map(|id| (id.clone(), true)).collect::<Vec<_>>()
    );
    for id in &ids {
        assert!(
            crate::store::effective::effective_receipt(&conn, id, "s")
                .unwrap()
                .is_some_and(|r| r.state == crate::store::effective::EffectiveReceiptState::Pending),
            "{id} still pending"
        );
    }
}

// `register_available` only ever runs for a top-level permit, so the daemon fast-forwards a
// subagent binding only through `fast_forward` itself: it moves that binding's own cursors and
// nothing else.
#[test]
fn subagent_fast_forward_never_settles() {
    let (store, conn, first, ids) = resumed(300, 60);
    let _ = store;
    let parent = binding_id(&conn, "sess-A");
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,role,parent_binding,agent_id) VALUES ('s',0,'p','b',1,0,'codex','sess-A',?1,'cooperative_top_level',0,0,'term-p','inc','subagent',?2,'ag1')",
        params![uuid::Uuid::new_v4().to_string(), parent.as_str()],
    )
    .unwrap();
    let sub = BindingId::new(
        conn.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE agent_id='ag1'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    );
    crate::store::read_cursors::init_for_new_binding(&conn, &sub).unwrap();
    assert_eq!(
        crate::store::read_cursors::cursor_for(&conn, &sub, &ThreadId::new("t")).unwrap(),
        Some(1),
        "it starts at the seat maximum, behind the backlog"
    );
    let receipts = |conn: &Connection| -> (i64, i64) {
        conn.query_row(
            "SELECT (SELECT count(*) FROM receipts),(SELECT count(*) FROM receipt_state)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    let before = receipts(&conn);
    let ff = fast_forward(&conn, &sub)
        .unwrap()
        .expect("the subagent's own backlog is over a page");
    assert_eq!(ff.skipped as usize, ids.len());
    let head = crate::store::read_cursors::thread_head(&conn, &ThreadId::new("t")).unwrap();
    assert_eq!(
        crate::store::read_cursors::cursor_for(&conn, &sub, &ThreadId::new("t")).unwrap(),
        Some(head)
    );
    // Only the subagent moved; the parent still has its backlog, and nothing was settled.
    assert_eq!(cursor(&conn, "sess-A"), Some(1));
    assert_eq!(receipts(&conn), before, "no receipt write");
    for id in &ids {
        assert!(
            crate::store::effective::effective_receipt(&conn, id, "s")
                .unwrap()
                .is_some_and(|r| r.state == crate::store::effective::EffectiveReceiptState::Pending),
            "{id} still pending"
        );
    }
    let _ = first;
}

#[test]
fn plain_touch_never_fast_forwards() {
    let (store, conn, first, _) = resumed(300, 60);
    // A plain call (Current mode) by the disconnected-but-open binding is a touch.
    let current = crate::protocol::commands::CheckIn {
        lineage_parent: None,
        mode: crate::protocol::commands::CheckInMode::Current,
        claim: first.context.clone(),
        operation: OperationId::new("touch"),
    };
    let touched = check_in(&store, current).unwrap();
    assert!(touched.fast_forward.is_none() && touched.first_contact.is_none());
    assert_eq!(cursor(&conn, "sess-A"), Some(1), "the cursor stayed");
    // A lifecycle check-in is what resumes it.
    assert!(resume(&store, &first, "r1").fast_forward.is_some());
}

#[test]
fn supersede_never_fast_forwards_the_superseded_binding() {
    let (store, conn) = seated(1);
    let pid = Some(LivenessFingerprint {
        pid: 4242,
        start: 7,
    });
    let mut a = session_claim("sess-A", EXEC_A);
    a.liveness = pid;
    let first = check_in(&store, lifecycle(a, "a1")).unwrap();
    for seq in 2..62 {
        deliver(&conn, "t", seq, "s", &"x".repeat(300));
    }
    // The same process starts another session (Claude /clear): sess-A is superseded.
    let mut b = session_claim("sess-B", EXEC_B);
    b.liveness = pid;
    let second = check_in(&store, lifecycle(b, "b1")).unwrap();
    let liveness: String = conn
        .query_row(
            "SELECT liveness FROM occupant_bindings WHERE native_session='sess-A'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(liveness, "disconnected", "superseded");
    assert!(second.fast_forward.is_none());
    assert_eq!(
        cursor(&conn, "sess-A"),
        Some(1),
        "the superseded binding's cursors are untouched"
    );
    // Only its own lifecycle resume fast-forwards it.
    assert!(resume(&store, &first, "a2").fast_forward.is_some());
}

fn v2_items(
    store: &crate::store::SqliteStore,
    claim: &CallerClaim,
    limit: u16,
) -> Vec<InboxBatchV2Item> {
    let CommandResult::InboxBatchV2(page) = query_with_output(
        &store.context,
        "i",
        &Command::InboxBatchV2(InboxQuery {
            claim: Some(claim.clone()),
            seat: Some(claim.seat.clone()),
            page: PageRequest {
                limit,
                max_bytes: 60_000,
                cursor: None,
            },
        }),
        &OutputSpec::default(),
        &budget(),
    )
    .unwrap() else {
        panic!("v2 page")
    };
    page.items
}

#[test]
fn unenrolled_inbox_page_carries_stats_until_the_enrolling_advance() {
    let (store, conn) = seated(5);
    let claim = session_claim("sess-A", EXEC_A);
    // A session the daemon has not enrolled: the page a fresh binding would get, with the stats
    // it would start from (the thread head). The read enrolls nothing.
    let items = v2_items(&store, &claim, 20);
    let Some(InboxBatchV2Item::FirstContact(fc)) = items.first() else {
        panic!("first-contact stats lead the page: {items:?}")
    };
    assert_eq!(
        (
            fc.threads[0].thread.as_str(),
            fc.threads[0].earlier,
            fc.threads[0].from_people
        ),
        ("t", 5, 5)
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    // A one-row page has no room for them, and a continuation page never repeats them.
    assert!(
        !v2_items(&store, &claim, 1)
            .iter()
            .any(|i| matches!(i, InboxBatchV2Item::FirstContact(_)))
    );
    // The CLI's advance after the display (here with no message shown) enrolls the binding...
    store
        .advance_cursors(
            &crate::protocol::commands::AdvanceCursors {
                claim: claim.clone(),
                operation: OperationId::new("enroll"),
                displayed: vec![],
            },
            &budget(),
        )
        .unwrap();
    assert_eq!(cursor(&conn, "sess-A"), Some(5));
    // ...and the page for the now-bound session has none (real seats start at generation 1).
    conn.execute("UPDATE occupant_bindings SET generation=1", [])
        .unwrap();
    conn.execute("UPDATE seats SET generation=1 WHERE id='s'", [])
        .unwrap();
    let mut bound = claim.clone();
    bound.binding_generation = 1;
    assert!(
        !v2_items(&store, &bound, 20)
            .iter()
            .any(|i| matches!(i, InboxBatchV2Item::FirstContact(_)))
    );
}
