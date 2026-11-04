use super::*;
use crate::protocol::time::UtcMillis;
use crate::protocol::time::{Clock, MonoInstant};
use crate::store::connection::StoreContext;
use crate::store::receipts::{self, ReceiptDueCursor};
use crate::sync::frame::LaunchState;
use std::sync::Arc;

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";
const TARGET: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const T0: i64 = 1_000_000;

/// Local author seat `s1`.
fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    crate::test_support::attach_test_host_to_connection(&db);
    db.execute_batch(&format!(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) \
         VALUES('s1','{INSTANCE}','resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));"
    ))
    .unwrap();
    db
}

fn req(n: u8) -> String {
    format!("{n:0>36}")
}

fn msg(n: u8) -> String {
    format!("m{n}")
}

fn insert(db: &Connection, n: u8, deadline_at: Option<i64>, bytes: &[u8]) -> (Obligation, bool) {
    let (request, message) = (req(n), msg(n));
    insert_or_get(
        db,
        &NewObligation {
            request: &request,
            target_node: TARGET,
            author: "s1",
            message: &message,
            requested_at: T0,
            deadline_at,
            request_bytes: bytes,
        },
    )
    .unwrap()
}

fn version(db: &Connection) -> Option<i64> {
    db.query_row(
        "SELECT attention_version FROM wake_work WHERE seat_id='s1'",
        [],
        |r| r.get(0),
    )
    .ok()
}

fn state(db: &Connection, n: u8) -> State {
    get(db, &req(n)).unwrap().unwrap().state
}

/// Puts row `n` into `target` by the public transitions only.
fn put_in(db: &mut Connection, n: u8, target: State) {
    let r = req(n);
    match target {
        State::Sent => {}
        State::Launched => assert!(launched(db, &r, "seat-x", T0 + 1).unwrap()),
        State::Linked => assert!(link(db, &msg(n), "seat-x", T0 + 1).unwrap()),
        State::Failed => assert!(fail(db, &r, "{}", false, T0 + 1).unwrap()),
        State::Expired => {
            assert_eq!(expire_due(db, T0 + OBLIGATION_TTL_MS).unwrap().len(), 1);
        }
    }
    assert_eq!(state(db, n), target);
}

#[test]
fn first_insert_freezes_bytes() {
    let db = fixture();
    let (first, inserted) = insert(&db, 1, None, b"A");
    assert!(inserted);
    assert_eq!(first.request_bytes, b"A");
    assert_eq!(first.state, State::Sent);
    assert_eq!(first.created_at, T0);
    assert_eq!(first.updated_at, T0);
    assert_eq!(first.expires_at, T0 + OBLIGATION_TTL_MS);
    let (again, inserted) = insert(&db, 1, Some(5), b"B");
    assert!(!inserted);
    assert_eq!(again, first);
    assert_eq!(again.deadline_at, None);
    assert_eq!(by_message(&db, "m1").unwrap().unwrap(), first);
}

#[test]
fn transition_table() {
    use State::*;
    let all = [Sent, Launched, Linked, Expired, Failed];
    // (event, states it changes, the state it leaves behind)
    for from in all {
        // launched: only from sent.
        let mut db = fixture();
        insert(&db, 1, None, b"A");
        put_in(&mut db, 1, from);
        let changed = launched(&db, &req(1), "seat-y", T0 + 5).unwrap();
        assert_eq!(changed, from == Sent, "launched from {from:?}");
        assert_eq!(state(&db, 1), if from == Sent { Launched } else { from });

        // link: from sent or launched.
        let mut db = fixture();
        insert(&db, 1, None, b"A");
        put_in(&mut db, 1, from);
        let changed = link(&db, &msg(1), "seat-y", T0 + 5).unwrap();
        assert_eq!(
            changed,
            matches!(from, Sent | Launched),
            "link from {from:?}"
        );
        assert_eq!(
            state(&db, 1),
            if changed { Linked } else { from },
            "link from {from:?}"
        );

        // fail: only from sent.
        let mut db = fixture();
        insert(&db, 1, None, b"A");
        put_in(&mut db, 1, from);
        let changed = fail(&db, &req(1), "{\"a\":1}", false, T0 + 5).unwrap();
        assert_eq!(changed, from == Sent, "fail from {from:?}");
        assert_eq!(state(&db, 1), if changed { Failed } else { from });

        // expiry: from sent or launched.
        let mut db = fixture();
        insert(&db, 1, None, b"A");
        put_in(&mut db, 1, from);
        let expired = expire_due(&db, T0 + OBLIGATION_TTL_MS + 5).unwrap();
        assert_eq!(
            expired.len(),
            usize::from(matches!(from, Sent | Launched)),
            "expiry from {from:?}"
        );
        assert_eq!(
            state(&db, 1),
            if matches!(from, Sent | Launched) {
                Expired
            } else {
                from
            }
        );
    }
}

#[test]
fn terminal_rows_keep_every_column() {
    let mut db = fixture();
    insert(&db, 1, None, b"A");
    put_in(&mut db, 1, State::Linked);
    let before = get(&db, &req(1)).unwrap().unwrap();
    assert!(!launched(&db, &req(1), "other", T0 + 9).unwrap());
    assert!(!link(&db, &msg(1), "other", T0 + 9).unwrap());
    assert!(!fail(&db, &req(1), "x", true, T0 + 9).unwrap());
    assert!(
        expire_due(&db, T0 + 2 * OBLIGATION_TTL_MS)
            .unwrap()
            .is_empty()
    );
    assert_eq!(get(&db, &req(1)).unwrap().unwrap(), before);
}

#[test]
fn link_by_unknown_message_is_a_no_op() {
    let db = fixture();
    insert(&db, 1, None, b"A");
    assert!(!link(&db, "nope", "seat-x", T0).unwrap());
    assert_eq!(state(&db, 1), State::Sent);
}

#[test]
fn fail_stores_answer_and_shown_only_when_returned() {
    let db = fixture();
    insert(&db, 1, None, b"A");
    insert(&db, 2, None, b"A");
    assert!(fail(&db, &req(1), "{\"refused\":1}", true, T0 + 7).unwrap());
    assert!(fail(&db, &req(2), "{\"failed\":1}", false, T0 + 8).unwrap());
    let shown = get(&db, &req(1)).unwrap().unwrap();
    assert_eq!(shown.answer.as_deref(), Some("{\"refused\":1}"));
    assert_eq!(shown.alert_shown_at, Some(T0 + 7));
    assert_eq!(shown.updated_at, T0 + 7);
    let hidden = get(&db, &req(2)).unwrap().unwrap();
    assert_eq!(hidden.answer.as_deref(), Some("{\"failed\":1}"));
    assert_eq!(hidden.alert_shown_at, None);
}

#[test]
fn overdue_is_columns() {
    let db = fixture();
    insert(&db, 1, Some(T0 + 10), b"A");
    assert_eq!(version(&db), None);
    assert!(mark_overdue(&db, &req(1), T0 + 20).unwrap());
    let row = get(&db, &req(1)).unwrap().unwrap();
    assert!(row.overdue_seq.is_some());
    assert_eq!(row.state, State::Sent);
    let first_seq = row.overdue_seq;
    assert!(!mark_overdue(&db, &req(1), T0 + 30).unwrap());
    assert_eq!(get(&db, &req(1)).unwrap().unwrap().overdue_seq, first_seq);
    assert_eq!(resend_set(&db, None).unwrap().len(), 1);
    assert_eq!(version(&db), Some(1));
    assert!(!mark_overdue(&db, &req(9), T0).unwrap());
    // A launched row whose pair thread is not held is an alert too: the overdue bumps once,
    // the launch itself bumps nothing.
    insert(&db, 2, Some(T0 + 10), b"A");
    assert!(launched(&db, &req(2), "seat-x", T0 + 1).unwrap());
    assert_eq!(version(&db), Some(1));
    assert!(mark_overdue(&db, &req(2), T0 + 20).unwrap());
    assert_eq!(version(&db), Some(2));
    // The sequence only rises.
    assert!(get(&db, &req(2)).unwrap().unwrap().overdue_seq > first_seq);
}

#[test]
fn thread_warned_is_set_once() {
    let db = fixture();
    insert(&db, 1, Some(T0), b"A");
    assert!(mark_thread_warned(&db, &req(1), T0 + 1).unwrap());
    assert!(!mark_thread_warned(&db, &req(1), T0 + 2).unwrap());
    assert_eq!(
        get(&db, &req(1)).unwrap().unwrap().thread_warned_at,
        Some(T0 + 1)
    );
}

#[test]
fn expiry_with_and_without_deadline() {
    let db = fixture();
    insert(&db, 1, None, b"A");
    insert(&db, 2, Some(T0 + 100), b"A");
    insert(&db, 3, Some(T0 + OBLIGATION_TTL_MS), b"A");
    insert(&db, 4, None, b"A");
    // Before expiry nothing moves.
    assert!(
        expire_due(&db, T0 + OBLIGATION_TTL_MS - 1)
            .unwrap()
            .is_empty()
    );
    assert_eq!(resend_set(&db, None).unwrap().len(), 4);
    let expired = expire_due(&db, T0 + OBLIGATION_TTL_MS).unwrap();
    assert_eq!(expired.len(), 4);
    assert!(expired.iter().all(|o| o.state == State::Expired));
    assert!(resend_set(&db, None).unwrap().is_empty());
    // No deadline: no overdue. Deadline reached by expiry: marked in the same call.
    assert_eq!(get(&db, &req(1)).unwrap().unwrap().overdue_seq, None);
    assert!(get(&db, &req(3)).unwrap().unwrap().overdue_seq.is_some());
    assert!(get(&db, &req(2)).unwrap().unwrap().overdue_seq.is_some());
    // A second pass expires nothing again.
    assert!(
        expire_due(&db, T0 + OBLIGATION_TTL_MS + 1)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn expiry_does_not_mark_overdue_before_the_deadline() {
    let db = fixture();
    // The deadline is set past expiry (not possible for a capped send, but the rule is on the clock).
    insert(&db, 1, Some(T0 + OBLIGATION_TTL_MS + 1), b"A");
    assert_eq!(expire_due(&db, T0 + OBLIGATION_TTL_MS).unwrap().len(), 1);
    assert_eq!(get(&db, &req(1)).unwrap().unwrap().overdue_seq, None);
}

#[test]
fn resend_set_filters_by_target_and_keeps_overdue_rows() {
    let mut db = fixture();
    insert(&db, 1, Some(T0), b"A");
    insert(&db, 2, None, b"A");
    insert(&db, 3, None, b"A");
    put_in(&mut db, 2, State::Launched);
    assert!(link(&db, &msg(3), "seat-z", T0 + 1).unwrap());
    mark_overdue(&db, &req(1), T0 + 5).unwrap();
    let all: Vec<String> = resend_set(&db, None)
        .unwrap()
        .into_iter()
        .map(|o| o.request)
        .collect();
    assert_eq!(all, vec![req(1), req(2)]);
    assert_eq!(resend_set(&db, Some(TARGET)).unwrap().len(), 2);
    assert!(
        resend_set(&db, Some("cccccccccccccccccccccccccccccccc"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn pending_alerts_carry_the_launched_seat() {
    let db = fixture();
    insert(&db, 1, Some(T0), b"A");
    insert(&db, 2, Some(T0), b"A");
    assert!(launched(&db, &req(2), "sZZZseat0001", T0 + 1).unwrap());
    mark_overdue(&db, &req(1), T0 + 2).unwrap();
    mark_overdue(&db, &req(2), T0 + 2).unwrap();
    let seats: Vec<(String, Option<String>)> = pending_alerts(&db, "s1")
        .unwrap()
        .into_iter()
        .map(|(request, _, _, seat)| (request, seat))
        .collect();
    assert_eq!(
        seats,
        vec![(req(1), None), (req(2), Some("sZZZseat0001".to_owned()))]
    );
}

#[test]
fn alert_predicate_survives_expiry_and_failure() {
    let db = fixture();
    insert(&db, 1, Some(T0), b"A");
    insert(&db, 2, Some(T0), b"A");
    mark_overdue(&db, &req(1), T0 + 1).unwrap();
    mark_overdue(&db, &req(2), T0 + 1).unwrap();
    let pending = pending_alerts(&db, "s1").unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(
        (pending[0].0.as_str(), pending[0].1.as_str()),
        (req(1).as_str(), TARGET)
    );
    assert!(pending[0].2 < pending[1].2);
    assert_eq!(alert_count(&db, "s1").unwrap(), 2);
    assert_eq!(latest_alert_seq(&db, "s1").unwrap(), Some(pending[1].2));

    // Failure and expiry do not clear the alert and bump nothing.
    let v = version(&db);
    assert!(fail(&db, &req(1), "{}", false, T0 + 2).unwrap());
    assert_eq!(expire_due(&db, T0 + OBLIGATION_TTL_MS).unwrap().len(), 1);
    assert_eq!(alert_count(&db, "s1").unwrap(), 2);
    assert_eq!(version(&db), v);
    assert_eq!(state(&db, 1), State::Failed);
    assert_eq!(state(&db, 2), State::Expired);

    // Pruning removes both and wakes the author once.
    assert_eq!(
        prune(&db, T0 + OBLIGATION_TTL_MS + PRUNE_AFTER_MS - 1).unwrap(),
        1
    );
    assert_eq!(alert_count(&db, "s1").unwrap(), 1);
    assert_eq!(
        prune(&db, T0 + OBLIGATION_TTL_MS + PRUNE_AFTER_MS + 1).unwrap(),
        1
    );
    assert_eq!(alert_count(&db, "s1").unwrap(), 0);
    assert!(version(&db) > v);
}

#[test]
fn alert_exits_bump_the_author() {
    let db = fixture();
    for n in 1..=3 {
        insert(&db, n, Some(T0), b"A");
        mark_overdue(&db, &req(n), T0 + 1).unwrap();
    }
    let v = version(&db).unwrap();
    // A launch alone is not an exit: the pair thread is not held, so the alert stays.
    assert!(launched(&db, &req(1), "seat-x", T0 + 2).unwrap());
    assert_eq!(alert_count(&db, "s1").unwrap(), 3);
    let v1 = version(&db).unwrap();
    assert_eq!(
        v1, v,
        "a launch that leaves the predicate unchanged bumps nothing"
    );
    assert!(link(&db, &msg(2), "seat-x", T0 + 3).unwrap());
    assert_eq!(alert_count(&db, "s1").unwrap(), 2);
    let v2 = version(&db).unwrap();
    assert_eq!(v2, v1 + 1, "a link out of the predicate bumps once");
    // Only the author's own pending rows are marked.
    assert_eq!(
        mark_alerts_shown(&db, "other", &[req(3)], T0 + 4).unwrap(),
        0
    );
    assert_eq!(version(&db), Some(v2));
    // A request that is not pending (already linked) is not marked.
    assert_eq!(
        mark_alerts_shown(&db, "s1", &[req(2), req(3)], T0 + 4).unwrap(),
        1
    );
    assert_eq!(alert_count(&db, "s1").unwrap(), 1, "the launched row stays");
    assert!(version(&db).unwrap() > v2);
    assert_eq!(
        get(&db, &req(3)).unwrap().unwrap().alert_shown_at,
        Some(T0 + 4)
    );
    assert_eq!(get(&db, &req(2)).unwrap().unwrap().alert_shown_at, None);
}

#[test]
fn a_retired_author_has_no_pending_alert() {
    let db = fixture();
    insert(&db, 1, Some(T0), b"A");
    mark_overdue(&db, &req(1), T0 + 1).unwrap();
    assert_eq!(alert_count(&db, "s1").unwrap(), 1);
    db.execute(
        "UPDATE seats SET state='retired', retired_at=1 WHERE id='s1'",
        [],
    )
    .unwrap();
    assert_eq!(alert_count(&db, "s1").unwrap(), 0);
    assert!(pending_alerts(&db, "s1").unwrap().is_empty());
    assert_eq!(latest_alert_seq(&db, "s1").unwrap(), None);
    assert_eq!(mark_alerts_shown(&db, "s1", &[req(1)], T0 + 2).unwrap(), 0);
}

#[test]
fn alert_query_uses_the_partial_index() {
    let db = fixture();
    let plan: Vec<String> = db
        .prepare(&format!(
            "EXPLAIN QUERY PLAN SELECT o.request FROM handoff_obligations o WHERE o.author='s1' AND {ALERT_PENDING_SQL} ORDER BY o.overdue_seq"
        ))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        plan.iter().any(|l| l.contains("handoff_obligations_alert")),
        "{plan:?}"
    );
}

#[test]
fn due_rows_cover_unseen_failed() {
    let db = fixture();
    insert(&db, 1, Some(T0 + 10), b"A"); // failed, unseen
    insert(&db, 2, Some(T0 + 10), b"A"); // failed, shown
    insert(&db, 3, Some(T0 + 10), b"A"); // sent, not overdue
    insert(&db, 4, Some(T0 + 10), b"A"); // sent, overdue, seat unknown
    insert(&db, 5, Some(T0 + 10), b"A"); // launched, overdue, unwarned
    insert(&db, 6, Some(T0 + 10), b"A"); // launched, warned
    insert(&db, 7, Some(T0 + 1000), b"A"); // deadline not reached
    insert(&db, 8, None, b"A"); // no deadline
    fail(&db, &req(1), "{}", false, T0).unwrap();
    fail(&db, &req(2), "{}", true, T0).unwrap();
    mark_overdue(&db, &req(4), T0).unwrap();
    launched(&db, &req(5), "seat-x", T0).unwrap();
    mark_overdue(&db, &req(5), T0).unwrap();
    launched(&db, &req(6), "seat-x", T0).unwrap();
    mark_overdue(&db, &req(6), T0).unwrap();
    mark_thread_warned(&db, &req(6), T0).unwrap();
    let due = |db: &Connection| -> Vec<String> {
        due_rows(db, T0 + 10, None, 100)
            .unwrap()
            .into_iter()
            .map(|o| o.request)
            .collect()
    };
    assert_eq!(due(&db), vec![req(1), req(3), req(5)]);
    // Once the failed row is marked overdue, it is not due again.
    mark_overdue(&db, &req(1), T0 + 11).unwrap();
    assert_eq!(due(&db), vec![req(3), req(5)]);
    assert_eq!(due_rows(&db, T0 + 10, None, 1).unwrap().len(), 1);
}

#[test]
fn prune_terminal_rows_only() {
    let db = fixture();
    for n in 1..=5 {
        insert(&db, n, None, b"A");
    }
    link(&db, &msg(1), "seat-x", T0 + 1).unwrap();
    fail(&db, &req(2), "{}", false, T0 + 1).unwrap();
    launched(&db, &req(4), "seat-x", T0 + 1).unwrap();
    // At exactly the cutoff nothing goes; one ms later the terminal rows do.
    assert_eq!(prune(&db, T0 + 1 + PRUNE_AFTER_MS).unwrap(), 0);
    assert_eq!(prune(&db, T0 + 2 + PRUNE_AFTER_MS).unwrap(), 2);
    assert!(get(&db, &req(1)).unwrap().is_none());
    assert!(get(&db, &req(2)).unwrap().is_none());
    for n in 3..=5 {
        assert!(get(&db, &req(n)).unwrap().is_some(), "open row {n} kept");
    }
    // Expired rows prune from their expiry time.
    assert_eq!(expire_due(&db, T0 + OBLIGATION_TTL_MS).unwrap().len(), 3);
    assert_eq!(
        prune(&db, T0 + OBLIGATION_TTL_MS + PRUNE_AFTER_MS).unwrap(),
        0
    );
    assert_eq!(
        prune(&db, T0 + OBLIGATION_TTL_MS + PRUNE_AFTER_MS + 1).unwrap(),
        3
    );
}

#[test]
fn ttl_below_retention() {
    const { assert!(OBLIGATION_TTL_MS < peer_requests::RETENTION_MS) };
}

#[test]
fn state_round_trips_and_rejects_unknown() {
    for s in [
        State::Sent,
        State::Launched,
        State::Linked,
        State::Expired,
        State::Failed,
    ] {
        assert_eq!(State::parse(s.as_str()), Some(s));
    }
    assert_eq!(State::parse("overdue"), None);
}

// Pre-link overdue (ht-8lem.14): the due step, the in-thread warning, the dedupe across linking
// and the ledger lines of `peer list`.

const RCPT: &str = "r1";
const FOREIGN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const D: i64 = T0 + 10;
const LATE: i64 = T0 + 20;

fn pair() -> String {
    direct_thread_id(&SeatId::new("s1"), &SeatId::new(RCPT))
        .unwrap()
        .as_str()
        .to_owned()
}

/// The target seat `r1` and the pair thread of `s1` and `r1`, on a connection that already holds `s1`.
fn seed_pair(db: &Connection) {
    db.pragma_update(None, "foreign_keys", "OFF").unwrap();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) \
         VALUES(?1,?2,'resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
        params![RCPT, INSTANCE],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) \
         VALUES (?1,?2,'topic','goal',0,0,0)",
        params![pair(), INSTANCE],
    )
    .unwrap();
}

/// `fixture()` plus the target seat `r1` and the pair thread of `s1` and `r1`.
fn pair_fixture() -> Connection {
    let db = fixture();
    seed_pair(&db);
    db
}

struct Fixed(i64);
impl Clock for Fixed {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}

/// A file-backed store whose clock stands at `LATE`, with `s1`, `r1` and the pair thread: the due
/// pass (`receipts::scan_due`, `scan_extension_lapses`) runs on it exactly as the daemon runs it.
fn due_fixture() -> (StoreContext, Connection) {
    let path =
        crate::test_support::scratch::temp_path(format!("handoff-due-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path, Arc::new(Fixed(LATE))).with_test_host();
    let db = context.open_writer().unwrap();
    db.pragma_update(None, "foreign_keys", "OFF").unwrap();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) \
         VALUES('s1',?1,'resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
        [INSTANCE],
    )
    .unwrap();
    seed_pair(&db);
    (context, db)
}

fn local_node(db: &Connection) -> String {
    db.query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .unwrap()
}

/// A pair-thread message `id` from `s1`, authored by `origin` (`None`: this node), whose deadline
/// belongs to `deadline_node`.
fn pair_message(db: &Connection, id: &str, origin: Option<&str>, deadline_node: Option<&str>) {
    db.execute(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq,origin_node,deadline_node) \
         VALUES (?1,?2,?3,(SELECT next_sequence FROM threads WHERE id=?3),'ordinary','s1','x',0,\
                 (SELECT decision_seq+1 FROM local_node),COALESCE(?4,(SELECT id FROM local_node)),?5)",
        params![id, INSTANCE, pair(), origin, deadline_node],
    )
    .unwrap();
    db.execute("UPDATE local_node SET decision_seq=decision_seq+1", [])
        .unwrap();
    db.execute(
        "UPDATE threads SET next_sequence=next_sequence+1 WHERE id=?1",
        [pair()],
    )
    .unwrap();
}

/// A pair-thread message `id` from `s1` with a pending ACK-required physical receipt for `r1`.
fn message_with_receipt(
    db: &Connection,
    id: &str,
    origin: Option<&str>,
    deadline_node: Option<&str>,
) {
    pair_message(db, id, origin, deadline_node);
    db.execute(
        "INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) \
         VALUES (?1,?2,?3,'pending',500,0,?4)",
        params![id, pair(), RCPT, D],
    )
    .unwrap();
}

/// Like `message_with_receipt`, but the receipt lives in `receipt_state`, where the sender
/// materializes a replicated message's receipt (`project_receipt`); the `receipt_due_sparse`
/// triggers index it.
fn message_with_sparse_receipt(
    db: &Connection,
    id: &str,
    origin: Option<&str>,
    deadline_node: Option<&str>,
) {
    pair_message(db, id, origin, deadline_node);
    db.execute(
        "INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at,ack_required) \
         VALUES (?1,?2,'pending',0,?3,1)",
        params![id, RCPT, D],
    )
    .unwrap();
}

/// `event_key`s of the overdue warnings in the store, sorted.
fn overdue_keys(db: &Connection) -> Vec<String> {
    let mut stmt = db
        .prepare(
            "SELECT event_key FROM messages WHERE event_key LIKE 'overdue:%' ORDER BY event_key",
        )
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The sender's view of a linked remote handoff message: the target authored it, this node owns
/// the deadline. Production shape.
fn sender_view(db: &Connection, id: &str, sparse: bool) {
    let local = local_node(db);
    let make = if sparse {
        message_with_sparse_receipt
    } else {
        message_with_receipt
    };
    make(db, id, Some(FOREIGN), Some(&local));
}

/// The target's view of the same message: this node authored it, the deadline is the sender's.
fn target_view(db: &Connection, id: &str, sparse: bool) {
    let make = if sparse {
        message_with_sparse_receipt
    } else {
        message_with_receipt
    };
    make(db, id, None, Some(FOREIGN));
}

fn full_due_scan(context: &StoreContext, db: &mut Connection) -> u16 {
    let mut cursor = ReceiptDueCursor::default();
    receipts::scan_due(context, db, 10, &mut cursor)
        .unwrap()
        .warnings
}

fn step(db: &Connection, n: u8, now: i64) -> (bool, Vec<String>) {
    let row = get(db, &req(n)).unwrap().unwrap();
    let mut lines = Vec::new();
    let inserted = due_step(db, &row, now, &mut |l| lines.push(l.to_owned()))
        .unwrap()
        .warned;
    (inserted, lines)
}

fn handoff_warnings(db: &Connection, n: u8) -> Vec<(String, String, String)> {
    let mut stmt = db
        .prepare("SELECT thread_id,kind,event_json FROM messages WHERE event_key=?1")
        .unwrap();
    stmt.query_map([format!("overdue:handoff:{}", req(n))], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn warn_count(db: &Connection) -> i64 {
    db.query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn due_seat_unknown_sets_overdue_seq_only() {
    let db = pair_fixture();
    insert(&db, 1, Some(D), b"A");
    // Before the deadline nothing moves.
    assert_eq!(step(&db, 1, D - 1), (false, vec![]));
    assert_eq!(get(&db, &req(1)).unwrap().unwrap().overdue_seq, None);
    let (inserted, lines) = step(&db, 1, LATE);
    assert!(!inserted);
    assert_eq!(
        lines,
        vec!["handoff 00000000 to bbbbbbbb overdue: not confirmed by that machine".to_owned()]
    );
    let row = get(&db, &req(1)).unwrap().unwrap();
    assert!(row.overdue_seq.is_some());
    assert_eq!(row.state, State::Sent);
    assert_eq!(row.thread_warned_at, None);
    assert_eq!(warn_count(&db), 0);
    assert_eq!(pending_alerts(&db, "s1").unwrap().len(), 1);
    // A second pass changes nothing and logs nothing.
    assert_eq!(step(&db, 1, LATE + 5), (false, vec![]));
    assert_eq!(get(&db, &req(1)).unwrap().unwrap(), row);
}

#[test]
fn due_seat_known_records_one_thread_warning() {
    let db = pair_fixture();
    insert(&db, 1, Some(D), b"A");
    assert!(launched(&db, &req(1), RCPT, T0 + 1).unwrap());
    let (inserted, lines) = step(&db, 1, LATE);
    assert!(inserted);
    assert_eq!(lines, Vec::<String>::new(), "a known seat logs nothing");
    let warnings = handoff_warnings(&db, 1);
    assert_eq!(warnings.len(), 1);
    let (thread, kind, payload) = &warnings[0];
    assert_eq!((thread.as_str(), kind.as_str()), (pair().as_str(), "warn"));
    let mut value: serde_json::Value = serde_json::from_str(payload).unwrap();
    assert_eq!(value["obligation"], "handoff");
    assert_eq!(value["request"], req(1));
    // The receipt overdue payload plus `,"request":"<uuid>"`: 49 bytes.
    value.as_object_mut().unwrap().remove("request");
    value["obligation"] = "receipt".into();
    assert_eq!(payload.len(), value.to_string().len() + 49);
    let (kind, condition, seat, clear): (String, String, String, Option<String>) = db
        .query_row(
            "SELECT condition_kind,condition_id,affected_seat_id,clear_warning_id FROM warning_conditions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (kind.as_str(), condition.as_str(), seat.as_str()),
        ("receipt", RCPT, RCPT)
    );
    assert_eq!(clear, None);
    let row = get(&db, &req(1)).unwrap().unwrap();
    assert_eq!(row.thread_warned_at, Some(LATE));
    assert!(row.overdue_seq.is_some());
    // The seat is known: no seat-addressed alert.
    assert!(pending_alerts(&db, "s1").unwrap().is_empty());
    // A second pass adds nothing.
    assert_eq!(step(&db, 1, LATE + 5), (false, vec![]));
    assert_eq!(handoff_warnings(&db, 1).len(), 1);
    assert!(due_rows(&db, LATE + 5, None, 10).unwrap().is_empty());
}

#[test]
fn seat_known_without_pair_thread_retries_later() {
    let db = fixture();
    insert(&db, 1, Some(D), b"A");
    assert!(launched(&db, &req(1), "seat-x", T0 + 1).unwrap());
    assert_eq!(step(&db, 1, LATE), (false, vec![]));
    let row = get(&db, &req(1)).unwrap().unwrap();
    assert!(row.overdue_seq.is_some());
    assert_eq!(row.thread_warned_at, None, "left for the next pass");
}

#[test]
fn late_launched_alert_swaps_for_thread_warning() {
    let mut db = pair_fixture();
    insert(&db, 1, Some(D), b"A");
    assert_eq!(due_pass(&mut db, LATE, 10).unwrap(), 0);
    assert_eq!(pending_alerts(&db, "s1").unwrap().len(), 1);
    assert_eq!(warn_count(&db), 0);
    assert!(launched(&db, &req(1), RCPT, LATE + 1).unwrap());
    // A late `Launched` alone does not clear the alert: the in-thread warning does.
    assert_eq!(pending_alerts(&db, "s1").unwrap().len(), 1);
    let before = version(&db);
    assert_eq!(due_pass(&mut db, LATE + 2, 10).unwrap(), 1);
    assert!(pending_alerts(&db, "s1").unwrap().is_empty());
    assert_eq!(version(&db), before.map(|v| v + 1), "the exit bumps once");
    assert_eq!(handoff_warnings(&db, 1).len(), 1);
    assert_eq!(due_pass(&mut db, LATE + 3, 10).unwrap(), 0);
    assert_eq!(warn_count(&db), 1);
}

#[test]
fn ack_clears_handoff_warning() {
    let mut db = pair_fixture();
    insert(&db, 1, Some(D), b"A");
    launched(&db, &req(1), RCPT, T0 + 1).unwrap();
    assert_eq!(due_pass(&mut db, LATE, 10).unwrap(), 1);
    // The receipt condition the ACK of the seat in the pair thread closes.
    let tx = db.transaction().unwrap();
    assert_eq!(
        schema::clear_warning_conditions_for_receipts(&tx, &pair(), RCPT, UtcMillis(LATE + 1))
            .unwrap(),
        1
    );
    tx.commit().unwrap();
    let clear: Option<String> = db
        .query_row("SELECT clear_warning_id FROM warning_conditions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(clear.is_some());
}

#[test]
fn no_second_warning_after_link() {
    let (context, mut db) = due_fixture();
    insert(&db, 1, Some(D), b"A");
    launched(&db, &req(1), RCPT, T0 + 1).unwrap();
    assert_eq!(due_pass(&mut db, LATE, 10).unwrap(), 1);
    // The message replicates (the target authored it, this node owns its deadline): it links, and
    // its receipt is due by the ordinary scan, which must find the handoff warning standing.
    sender_view(&db, &msg(1), false);
    assert!(link(&db, &msg(1), RCPT, LATE + 1).unwrap());
    assert_eq!(full_due_scan(&context, &mut db), 0, "no second warning");
    assert_eq!(warn_count(&db), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM messages WHERE event_key LIKE 'overdue:receipt:%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    // The linked message's receipt stands behind the handoff warning: the seat's ACK of it,
    // not an ACK of nothing, clears the condition.
    let tx = db.transaction().unwrap();
    assert_eq!(
        schema::clear_warning_conditions_for_receipts(&tx, &pair(), RCPT, UtcMillis(LATE + 3))
            .unwrap(),
        0,
        "still pending"
    );
    tx.execute(
        "UPDATE receipts SET state='acked' WHERE message_id=?1",
        [msg(1)],
    )
    .unwrap();
    assert_eq!(
        schema::clear_warning_conditions_for_receipts(&tx, &pair(), RCPT, UtcMillis(LATE + 4))
            .unwrap(),
        1
    );
}

#[test]
fn cleared_prelink_warning_lets_the_linked_message_warn() {
    let (context, mut db) = due_fixture();
    insert(&db, 1, Some(D), b"A");
    launched(&db, &req(1), RCPT, T0 + 1).unwrap();
    assert_eq!(due_pass(&mut db, LATE, 10).unwrap(), 1);
    let prelink: String = db
        .query_row(
            "SELECT id FROM messages WHERE event_key=?1",
            [format!("overdue:handoff:{}", req(1))],
            |r| r.get(0),
        )
        .unwrap();
    // An ACK of another pair message closes the pre-link warning's receipt condition: nothing
    // pending points at it yet.
    message_with_receipt(&db, &msg(2), None, None);
    db.execute(
        "UPDATE receipts SET state='acked' WHERE message_id=?1",
        [msg(2)],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        schema::clear_warning_conditions_for_receipts(&tx, &pair(), RCPT, UtcMillis(LATE + 1))
            .unwrap(),
        1
    );
    tx.commit().unwrap();
    let cleared: Option<String> = db
        .query_row(
            "SELECT clear_warning_id FROM warning_conditions WHERE open_warning_id=?1",
            [&prelink],
            |r| r.get(0),
        )
        .unwrap();
    assert!(cleared.is_some(), "the ACK cleared the pre-link condition");
    // The handoff message replicates and links; its deadline has passed and nobody ACKed it.
    sender_view(&db, &msg(1), false);
    assert!(link(&db, &msg(1), RCPT, LATE + 2).unwrap());
    assert_eq!(
        full_due_scan(&context, &mut db),
        1,
        "the linked message warns again"
    );
    let keys = overdue_keys(&db);
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(keys[0].starts_with("overdue:handoff:"), "{keys:?}");
    assert!(keys[1].starts_with("overdue:receipt:"), "{keys:?}");
    let own: Option<String> = db
        .query_row(
            "SELECT warning_message_id FROM receipts WHERE message_id=?1",
            [msg(1)],
            |r| r.get(0),
        )
        .unwrap();
    assert!(own.is_some_and(|w| w != prelink), "its own warning");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM warning_conditions WHERE condition_kind='receipt' AND thread_id=?1 \
             AND condition_id=?2 AND clear_warning_id IS NULL",
            params![pair(), RCPT],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "a new open condition"
    );
    assert_eq!(
        full_due_scan(&context, &mut db),
        0,
        "a second scan adds nothing"
    );
}

#[test]
fn target_never_records() {
    let (context, mut db) = due_fixture();
    // The target's view: it authored the message, the deadline is the sender node's.
    target_view(&db, &msg(1), false);
    // The sender's view of another handoff message does go overdue through the same pass.
    sender_view(&db, &msg(2), false);
    // The target holds no ledger row for the request.
    let outcome = schema::record_overdue_if_pending(
        &db,
        &ObligationRef::Handoff(req(1)),
        &TimeBasis::Decision,
        UtcMillis(LATE),
    )
    .unwrap();
    assert!(!outcome.inserted);
    assert_eq!(warn_count(&db), 0);
    assert!(due_rows(&db, LATE, None, 10).unwrap().is_empty());
    assert_eq!(full_due_scan(&context, &mut db), 1);
    assert_eq!(
        overdue_keys(&db).len(),
        1,
        "only the sender's view is recorded"
    );
    assert_eq!(
        db.query_row(
            "SELECT source_message_id FROM messages WHERE event_key LIKE 'overdue:receipt:%'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        msg(2)
    );
}

#[test]
fn due_scan_owner_is_the_deadline_node() {
    let (context, mut db) = due_fixture();
    sender_view(&db, "m1", false);
    target_view(&db, "m2", false);
    assert_eq!(full_due_scan(&context, &mut db), 1);
    let keys = overdue_keys(&db);
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!(keys[0].starts_with("overdue:receipt:"), "{keys:?}");
    assert_eq!(warned_message(&db), ["m1"]);
}

fn sparse_index(db: &Connection) -> Vec<String> {
    db.prepare("SELECT message_id FROM receipt_due_sparse ORDER BY message_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The sparse path stops at the trigger-fed index in this fixture: a `receipt_state` row whose
/// message has no published receipt in the manifest is not an effective receipt, so the scan cannot
/// record it (the multinode `linked_then_overdue_on_the_sender_only` covers the sparse path end to
/// end). The index membership is the triggers' whole job.
#[test]
fn sparse_due_index_owner_is_the_deadline_node() {
    let (_context, db) = due_fixture();
    sender_view(&db, "m1", true);
    target_view(&db, "m2", true);
    assert_eq!(
        sparse_index(&db),
        ["m1"],
        "the insert trigger indexes the deadline owner's row only"
    );
    // The update trigger: rows that start acked and turn pending again.
    for (id, sender) in [("m3", true), ("m4", false)] {
        let local = local_node(&db);
        let (origin, node) = if sender {
            (Some(FOREIGN), local)
        } else {
            (None, FOREIGN.to_owned())
        };
        pair_message(&db, id, origin, Some(&node));
        db.execute(
            "INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at,ack_required) \
             VALUES (?1,?2,'acked',0,?3,1)",
            params![id, RCPT, D],
        )
        .unwrap();
        db.execute(
            "UPDATE receipt_state SET state='pending' WHERE message_id=?1",
            [id],
        )
        .unwrap();
    }
    assert_eq!(sparse_index(&db), ["m1", "m3"]);
}

#[test]
fn sparse_repair_insert_owner_is_the_deadline_node() {
    let (context, mut db) = due_fixture();
    sender_view(&db, "m1", true);
    target_view(&db, "m2", true);
    // Drop the index and re-open the discovery repair, so the scan rebuilds it from `receipt_state`.
    db.execute("DELETE FROM receipt_due_sparse", []).unwrap();
    db.execute(
        "UPDATE receipt_discovery_backfill SET sparse_after=0,completed=0,\
         sparse_high_water=(SELECT max(rowid) FROM receipt_state) WHERE singleton=1",
        [],
    )
    .unwrap();
    full_due_scan(&context, &mut db);
    assert_eq!(sparse_index(&db), ["m1"]);
}

#[test]
fn extension_lapse_owner_is_the_deadline_node() {
    let (context, mut db) = due_fixture();
    sender_view(&db, "m1", false);
    target_view(&db, "m2", false);
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,'p','b',0,'codex','s','e','cooperative_top_level',0,'tm','inc')", [RCPT]).unwrap();
    // The extension ended before the clock: the lapse recheck finds the receipts due.
    db.execute("INSERT INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,extension_until,state) SELECT binding_id,?1,?2,0,0,?3,'active' FROM occupant_bindings WHERE seat_id=?1", params![RCPT, pair(), D + 1]).unwrap();
    let mut cursor = ReceiptDueCursor::default();
    let lapses = receipts::scan_extension_lapses(&context, &mut db, &mut cursor).unwrap();
    assert_eq!(lapses.warnings, 1, "m1 only");
    assert_eq!(warned_message(&db), ["m1"]);
}

/// The second arm of the lapse query reads `receipt_state`. A target-view row is not selected
/// (the scan succeeds with no lapse); a sender-view row is selected and reaches the record step,
/// which fails here only because the fixture publishes no manifest receipt for it.
#[test]
fn extension_lapse_sparse_arm_owner_is_the_deadline_node() {
    let lapse = |sender: bool| {
        let (context, mut db) = due_fixture();
        if sender {
            sender_view(&db, "m1", true);
        } else {
            target_view(&db, "m1", true);
        }
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,'p','b',0,'codex','s','e','cooperative_top_level',0,'tm','inc')", [RCPT]).unwrap();
        db.execute("INSERT INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,extension_until,state) SELECT binding_id,?1,?2,0,0,?3,'active' FROM occupant_bindings WHERE seat_id=?1", params![RCPT, pair(), D + 1]).unwrap();
        let mut cursor = ReceiptDueCursor::default();
        receipts::scan_extension_lapses(&context, &mut db, &mut cursor)
    };
    assert_eq!(lapse(false).unwrap().warnings, 0);
    let selected = lapse(true).unwrap_err();
    assert_eq!(selected.detail, "receipt obligation missing");
}

/// The fragment text the Rust due filters share, rendered for `NEW.message_id`, as the triggers hold it.
#[test]
fn deadline_owner_triggers_use_the_fragment() {
    let db = fixture();
    let fragment = schema::deadline_owner_sql!("NEW.message_id");
    let squash = |sql: &str| sql.split_whitespace().collect::<Vec<_>>().join(" ");
    for trigger in ["receipt_due_sparse_insert", "receipt_due_sparse_update"] {
        let sql: String = db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [trigger],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            squash(&sql).contains(&squash(fragment)),
            "{trigger} drifted from the deadline-owner fragment: {sql}"
        );
    }
}

/// `source_message_id`s of the overdue receipt warnings, sorted.
fn warned_message(db: &Connection) -> Vec<String> {
    let mut stmt = db
        .prepare("SELECT source_message_id FROM messages WHERE event_key LIKE 'overdue:receipt:%' ORDER BY 1")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn expiry_runs_full_due_step() {
    let db = pair_fixture();
    insert(&db, 1, Some(D), b"A");
    insert(&db, 2, Some(D), b"A");
    launched(&db, &req(1), RCPT, T0 + 1).unwrap();
    let expired = expire_due(&db, T0 + OBLIGATION_TTL_MS).unwrap();
    assert_eq!(expired.len(), 2);
    // The launched row warned in its pair thread, then expired.
    assert_eq!(handoff_warnings(&db, 1).len(), 1);
    assert_eq!(state(&db, 1), State::Expired);
    assert!(
        get(&db, &req(1))
            .unwrap()
            .unwrap()
            .thread_warned_at
            .is_some()
    );
    // The sent row raised the alert, then expired; the alert survives expiry.
    let sent = get(&db, &req(2)).unwrap().unwrap();
    assert_eq!(sent.state, State::Expired);
    assert!(sent.overdue_seq.is_some());
    assert_eq!(pending_alerts(&db, "s1").unwrap().len(), 1);
    assert!(handoff_warnings(&db, 2).is_empty());
}

#[test]
fn unseen_failed_row_raises_alert_at_deadline() {
    let mut db = pair_fixture();
    insert(&db, 1, Some(D), b"A");
    insert(&db, 2, Some(D), b"A");
    assert!(fail(&db, &req(1), "{}", false, T0 + 1).unwrap());
    assert!(fail(&db, &req(2), "{}", true, T0 + 1).unwrap());
    assert_eq!(due_pass(&mut db, D - 1, 10).unwrap(), 0);
    assert_eq!(get(&db, &req(1)).unwrap().unwrap().overdue_seq, None);
    assert_eq!(due_pass(&mut db, LATE, 10).unwrap(), 0);
    assert!(get(&db, &req(1)).unwrap().unwrap().overdue_seq.is_some());
    assert_eq!(
        get(&db, &req(2)).unwrap().unwrap().overdue_seq,
        None,
        "a failure the author saw raises nothing"
    );
    let alerts = pending_alerts(&db, "s1").unwrap();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].0, req(1));
}

#[test]
fn ledger_rows_list_only_what_peer_list_shows() {
    let db = pair_fixture();
    for n in 1..=8 {
        insert(&db, n, Some(D), b"A");
    }
    // 1: sent, overdue, seat unknown (listed); 2: launched, overdue (the thread warns); 3: linked;
    // 4: plain sent (not overdue yet); 5: failed with an answer; 6, 7: expired; 8: sent overdue.
    link(&db, &msg(3), RCPT, T0 + 1).unwrap();
    launched(&db, &req(2), RCPT, T0 + 1).unwrap();
    launched(&db, &req(7), RCPT, T0 + 1).unwrap();
    let refused = serde_json::to_string(&RemoteLaunchAnswer::Answered {
        state: LaunchState::Refused {
            detail: "no such profile".into(),
        },
    })
    .unwrap();
    fail(&db, &req(5), &refused, true, T0 + 1).unwrap();
    db.execute(
        "UPDATE handoff_obligations SET deadline_at=NULL WHERE request=?1",
        [req(4)],
    )
    .unwrap();
    for n in [1, 2, 8] {
        step(&db, n, LATE);
    }
    db.execute(
        "UPDATE handoff_obligations SET expires_at=?1 WHERE request IN (?2,?3)",
        params![LATE, req(6), req(7)],
    )
    .unwrap();
    assert_eq!(expire_due(&db, LATE).unwrap().len(), 2);
    let rows = ledger_rows(&db).unwrap();
    let shown: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                &r.request[30..],
                r.state.as_str(),
                r.overdue,
                r.seat.as_deref(),
                r.detail.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        vec![
            ("000001", "sent", true, None, None),
            ("000005", "failed", false, None, Some("no such profile")),
            ("000006", "expired", true, None, None),
            ("000007", "expired", true, Some(RCPT), None),
            ("000008", "sent", true, None, None),
        ]
    );
    assert!(rows.iter().all(|r| r.node == "bbbbbbbb"));
}

/// Sets the columns the alert predicate reads on request `n`.
fn set_columns(
    db: &Connection,
    n: u8,
    state: &str,
    overdue: bool,
    warned: bool,
    shown: bool,
    seat: Option<&str>,
) {
    db.execute(
        "UPDATE handoff_obligations SET state=?2, overdue_seq=?3, thread_warned_at=?4, \
         alert_shown_at=?5, seat=?6 WHERE request=?1",
        params![
            req(n),
            state,
            overdue.then_some(7),
            warned.then_some(T0),
            shown.then_some(T0),
            seat
        ],
    )
    .unwrap();
}

#[test]
fn alert_predicate_table() {
    let db = fixture();
    // (state, overdue, warned, shown, seat, pending)
    type Case = (&'static str, bool, bool, bool, Option<&'static str>, bool);
    let table: [Case; 11] = [
        ("sent", true, false, false, None, true),
        ("sent", false, false, false, None, false),
        ("launched", true, false, false, Some("seat-x"), true),
        ("launched", true, true, false, Some("seat-x"), false),
        ("launched", true, false, true, Some("seat-x"), false),
        ("launched", false, false, false, Some("seat-x"), false),
        ("linked", true, false, false, Some("seat-x"), false),
        ("expired", true, false, false, Some("seat-x"), true),
        ("expired", true, true, false, Some("seat-x"), false),
        ("expired", true, false, false, None, true),
        ("failed", true, false, false, None, true),
    ];
    for (i, (state, overdue, warned, shown, seat, pending)) in table.iter().enumerate() {
        let n = i as u8 + 1;
        insert(&db, n, Some(T0), b"A");
        set_columns(&db, n, state, *overdue, *warned, *shown, *seat);
        let row = get(&db, &req(n)).unwrap().unwrap();
        assert_eq!(row.alert_pending(), *pending, "row {i}: {row:?}");
        let listed = pending_alerts(&db, "s1")
            .unwrap()
            .iter()
            .any(|(request, _, _, _)| *request == req(n));
        assert_eq!(
            listed, *pending,
            "SQL predicate disagrees on row {i}: {row:?}"
        );
        db.execute("DELETE FROM handoff_obligations", []).unwrap();
    }
}

#[test]
fn launching_a_pending_row_bumps_nothing_and_the_thread_warning_bumps_once() {
    let mut db = pair_fixture();
    insert(&db, 1, Some(D), b"A");
    assert_eq!(due_pass(&mut db, LATE, 10).unwrap(), 0);
    let v = version(&db);
    assert!(v.is_some());
    assert!(launched(&db, &req(1), RCPT, LATE + 1).unwrap());
    assert_eq!(version(&db), v, "the launch leaves the predicate unchanged");
    assert_eq!(due_pass(&mut db, LATE + 2, 10).unwrap(), 1);
    assert_eq!(
        version(&db),
        v.map(|v| v + 1),
        "the warning exit bumps once"
    );
    // Nothing left to do: a further pass bumps nothing.
    assert_eq!(due_pass(&mut db, LATE + 3, 10).unwrap(), 0);
    assert_eq!(version(&db), v.map(|v| v + 1));
}

#[test]
fn link_out_of_the_predicate_bumps_once() {
    let db = fixture();
    insert(&db, 1, Some(T0), b"A");
    assert!(launched(&db, &req(1), "seat-x", T0 + 1).unwrap());
    assert!(mark_overdue(&db, &req(1), T0 + 2).unwrap());
    let v = version(&db);
    assert!(link(&db, &msg(1), "seat-x", T0 + 3).unwrap());
    assert_eq!(version(&db), v.map(|v| v + 1));
    assert_eq!(alert_count(&db, "s1").unwrap(), 0);
}

#[test]
fn ledger_rows_show_launched_overdue_until_warned() {
    let db = fixture();
    insert(&db, 1, Some(T0), b"A");
    assert!(launched(&db, &req(1), "seat-x", T0 + 1).unwrap());
    assert!(ledger_rows(&db).unwrap().is_empty(), "not overdue yet");
    assert!(mark_overdue(&db, &req(1), T0 + 2).unwrap());
    let rows = ledger_rows(&db).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].state.as_str(),
            rows[0].seat.as_deref(),
            rows[0].overdue
        ),
        ("launched", Some("seat-x"), true)
    );
    assert!(mark_thread_warned(&db, &req(1), T0 + 3).unwrap());
    assert!(ledger_rows(&db).unwrap().is_empty());
}

/// Fails every update of request `n`, as a store fault would.
fn fail_updates_of(db: &Connection, n: u8) {
    db.execute_batch(&format!(
        "CREATE TRIGGER fail_one BEFORE UPDATE ON handoff_obligations WHEN OLD.request='{}' \
         BEGIN SELECT RAISE(ABORT,'injected'); END",
        req(n)
    ))
    .unwrap();
}

#[test]
fn due_pass_pages_past_rows_it_cannot_advance() {
    let mut db = fixture();
    // Twenty launched rows with their seat and overdue set, the pair thread not held: the due
    // step cannot move them, and they sort first.
    for n in 1..=20 {
        insert(&db, n, Some(T0 + 1), b"A");
        launched(&db, &req(n), "seat-x", T0).unwrap();
        mark_overdue(&db, &req(n), T0 + 2).unwrap();
    }
    insert(&db, 21, Some(T0 + 5), b"A");
    assert_eq!(due_pass(&mut db, T0 + 10, 16).unwrap(), 0);
    let late = get(&db, &req(21)).unwrap().unwrap();
    assert!(late.overdue_seq.is_some(), "the later row was reached");
    assert!(
        pending_alerts(&db, "s1")
            .unwrap()
            .iter()
            .any(|(r, _, _, _)| *r == req(21))
    );
    // Idempotent: a second pass moves nothing.
    let seq = late.overdue_seq;
    assert_eq!(due_pass(&mut db, T0 + 11, 16).unwrap(), 0);
    assert_eq!(get(&db, &req(21)).unwrap().unwrap().overdue_seq, seq);
}

#[test]
fn due_pass_stops_after_limit_advanced_rows() {
    let mut db = fixture();
    for n in 1..=5 {
        insert(&db, n, Some(T0 + i64::from(n)), b"A");
    }
    assert_eq!(due_pass(&mut db, T0 + 10, 2).unwrap(), 0);
    let marked = (1..=5)
        .filter(|n| get(&db, &req(*n)).unwrap().unwrap().overdue_seq.is_some())
        .count();
    assert_eq!(marked, 2, "at most `limit` rows advance per pass");
}

#[test]
fn due_pass_skips_a_failing_row() {
    let mut db = fixture();
    for n in 1..=3 {
        insert(&db, n, Some(T0 + 1), b"A");
    }
    fail_updates_of(&db, 2);
    assert_eq!(due_pass(&mut db, T0 + 10, 16).unwrap(), 0);
    let overdue = |n| get(&db, &req(n)).unwrap().unwrap().overdue_seq.is_some();
    assert!(overdue(1));
    assert!(!overdue(2), "the failing row rolled back");
    assert!(overdue(3));
}

#[test]
fn expire_due_skips_a_failing_row() {
    let db = fixture();
    for n in 1..=3 {
        insert(&db, n, Some(T0 + 1), b"A");
    }
    fail_updates_of(&db, 2);
    let expired = expire_due(&db, T0 + OBLIGATION_TTL_MS).unwrap();
    let ids: Vec<_> = expired.iter().map(|o| o.request.clone()).collect();
    assert_eq!(ids, vec![req(1), req(3)]);
    assert_eq!(state(&db, 2), State::Sent);
    assert_eq!(get(&db, &req(2)).unwrap().unwrap().overdue_seq, None);
}
