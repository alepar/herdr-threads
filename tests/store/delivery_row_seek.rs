//! `DeliveryRowSeek` (ht-yha.5.2): the exact ordered pending seek over `lazy_recipients`, and the
//! per-binding displayed marks (`binding_row_displays`) the unread predicates exclude. Included from
//! `src/store/lazy_delivery/inbox.rs` as `delivery_row_seek_tests`.
use super::*;
use crate::protocol::{ids::*, pagination::DeliveryRowsPosition, time::UtcMillis};
use crate::store::{
    bindings::{
        BindingRole, EnrollOutcome, EnrollRequest, EnrollTrigger, enroll,
        tests::{db, key, placement},
    },
    read_cursors,
};

const DH: u64 = 10_000;
/// Offset keeping fixture publication decisions clear of the shared fixture's messages 1 and 2.
const DBASE: i64 = 10;

fn enroll_binding(db: &Connection, session: &str, subagent_of: Option<&BindingId>) -> BindingId {
    let (k, p) = (key(session, subagent_of.map(|_| "agent-1")), placement());
    let req = EnrollRequest {
        key: &k,
        role: if subagent_of.is_some() {
            BindingRole::Subagent
        } else {
            BindingRole::TopLevel
        },
        parent: subagent_of,
        lineage: None,
        fingerprint: None,
        placement: &p,
        trigger: EnrollTrigger::Lifecycle,
    };
    match enroll(db, &req, UtcMillis(10)).unwrap() {
        EnrollOutcome::Enrolled { binding, .. } => binding.binding_id,
        other => panic!("{other:?}"),
    }
}

fn s() -> SeatId {
    SeatId::new("s")
}

fn t(name: &str) -> ThreadId {
    ThreadId::new(name)
}

/// One published delivery row for seat `s`: message `msg-<thread>-<n>` at sequence `n` of
/// `thread`, priority `p`, publication decision `d`.
struct Row {
    thread: &'static str,
    n: i64,
    p: u8,
    d: i64,
}

fn msg(thread: &str, n: i64) -> MessageId {
    MessageId::new(format!("msg-{thread}-{n}"))
}

fn add_thread(db: &Connection, thread: &str) {
    db.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0)",
        [thread],
    )
    .unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES (?1,'s','joined',1)",
        [thread],
    )
    .unwrap();
}

/// Publish through the same statements the send path ends with (staged row, message, manifest) and
/// `note_published`, which copies the decision onto the row.
fn publish(db: &mut Connection, rows: &[Row]) {
    db.execute(
        "INSERT OR IGNORE INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES ('a','34750f98bd59fcfc946da45aaabe933b','resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    for Row { thread, n, p, d } in rows {
        let d = &(d + DBASE);
        let id = format!("{thread}-{n}");
        let prep = format!("prep-{id}");
        tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','scope',?1,zeroblob(32),?2,0,0,0,0,0,0,0,'building','lazy',?3)",params![prep,thread,p]).unwrap();
        tx.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES(?1,?2,?3,'s',?4)",params![prep,format!("msg-{id}"),thread,p]).unwrap();
        tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,actor_seat_id,priority) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b',?2,?3,'ordinary',?4,'passive',0,'lazy','a',?5)",params![format!("msg-{id}"),thread,n,d,p]).unwrap();
        tx.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES(?1,?2,'34750f98bd59fcfc946da45aaabe933b',?3,?4,0,?4,0,1,0)",params![prep,format!("msg-{id}"),thread,d]).unwrap();
        note_published(&tx, &prep).unwrap();
    }
    tx.commit().unwrap();
}

fn seek(
    db: &Connection,
    filter: RowFilter<'_>,
    after: Option<DeliveryRowsPosition>,
    max_visits: usize,
) -> SeekResult {
    seek_pending(
        db,
        "34750f98bd59fcfc946da45aaabe933b",
        &s(),
        filter,
        after,
        DH,
        max_visits,
    )
    .unwrap()
}

/// Every row the filter accepts, in seek order, one visit at a time.
fn drain(db: &Connection, filter: RowFilter<'_>) -> Vec<MessageId> {
    let (mut after, mut out) = (None, Vec::new());
    loop {
        let r = seek(db, filter, after, 1);
        if let Some((_, recipient)) = r.row {
            out.push(recipient.message);
        }
        if r.exhausted {
            return out;
        }
        after = r.position;
    }
}

fn plan(db: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> String {
    db.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(params, |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join(" | ")
}

fn unread(db: &Connection, binding: &BindingId) -> Vec<MessageId> {
    binding_lazy_unread(db, binding, 100)
        .unwrap()
        .into_iter()
        .map(|u| u.message)
        .collect()
}

fn marks(db: &Connection, binding: &BindingId) -> Vec<String> {
    db.prepare(
        "SELECT message_id FROM binding_row_displays WHERE binding_id=?1 ORDER BY message_id",
    )
    .unwrap()
    .query_map([binding.as_str()], |r| r.get(0))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn complete(db: &mut Connection, binding: &BindingId, ids: &[MessageId]) {
    let tx = db.transaction().unwrap();
    complete_for_binding(
        &tx,
        "34750f98bd59fcfc946da45aaabe933b",
        &s(),
        Some(binding),
        ids,
    )
    .unwrap();
    tx.commit().unwrap();
}

#[test]
fn p0_row_behind_500_p1_rows_is_first_in_one_visit() {
    let mut db = db();
    enroll_binding(&db, "sess-A", None);
    let mut rows: Vec<Row> = (3..503)
        .map(|n| Row {
            thread: "t",
            n,
            p: 1,
            d: n,
        })
        .collect();
    rows.push(Row {
        thread: "t",
        n: 503,
        p: 0,
        d: 503,
    });
    publish(&mut db, &rows);
    let r = seek(&db, RowFilter::Seat, None, 100);
    let (position, recipient) = r.row.expect("a row");
    assert_eq!(
        recipient.message,
        msg("t", 503),
        "the p0 row, not the oldest p1"
    );
    assert_eq!(r.visited, 1, "no walk over the 500 p1 rows");
    assert_eq!(position.priority.get(), 0);
    assert_eq!(r.position, Some(position));
}

#[test]
fn unpublished_backlog_is_never_visited() {
    let mut db = db();
    enroll_binding(&db, "sess-A", None);
    db.execute("INSERT OR IGNORE INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES ('a','34750f98bd59fcfc946da45aaabe933b','resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    db.execute("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<10000) INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) SELECT 'prep-stg'||i,'34750f98bd59fcfc946da45aaabe933b','scope','prep-stg'||i,zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy',1 FROM n", []).unwrap();
    db.execute("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<10000) INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) SELECT 'prep-stg'||i,'msg-stg'||i,'t','s' FROM n", []).unwrap();
    let staged: i64 = db
        .query_row(
            "SELECT count(*) FROM lazy_recipients WHERE decision_seq IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(staged, 10_000);
    let r = seek(&db, RowFilter::Seat, None, 100);
    assert_eq!(
        (r.row, r.visited, r.exhausted),
        (None, 0, true),
        "nothing published, nothing visited"
    );
    publish(
        &mut db,
        &[Row {
            thread: "t",
            n: 3,
            p: 1,
            d: 3,
        }],
    );
    let r = seek(&db, RowFilter::Seat, None, 100);
    assert_eq!(r.row.unwrap().1.message, msg("t", 3));
    assert_eq!(r.visited, 1, "the 10k staged rows are outside the index");
    let seat = plan(
        &db,
        SEEK_VIEW_SQL,
        &[&"any-binding", &(DH as i64), &-1i64, &0i64, &0i64],
    );
    assert!(seat.contains("binding_pending_order"), "{seat}");
    assert!(!seat.contains("TEMP B-TREE"), "{seat}");
    let thread = plan(
        &db,
        SEEK_VIEW_THREAD_SQL,
        &[&"any-binding", &(DH as i64), &-1i64, &0i64, &0i64, &"t"],
    );
    assert!(thread.contains("binding_pending_thread_order"), "{thread}");
    assert!(!thread.contains("TEMP B-TREE"), "{thread}");
}

#[test]
fn resume_after_a_position() {
    let mut db = db();
    enroll_binding(&db, "sess-A", None);
    publish(
        &mut db,
        &[
            Row {
                thread: "t",
                n: 3,
                p: 1,
                d: 5,
            },
            Row {
                thread: "t",
                n: 4,
                p: 0,
                d: 6,
            },
            Row {
                thread: "t",
                n: 5,
                p: 1,
                d: 4,
            },
        ],
    );
    let mut after = None;
    let mut seen = Vec::new();
    for _ in 0..3 {
        let r = seek(&db, RowFilter::Seat, after, 1);
        seen.push(r.row.expect("a row").1.message);
        assert!(!r.exhausted);
        after = r.position;
    }
    assert_eq!(
        seen,
        [msg("t", 4), msg("t", 5), msg("t", 3)],
        "(priority, decision_seq, ordinal) order, not sequence order"
    );
    let r = seek(&db, RowFilter::Seat, after, 1);
    assert_eq!((r.row, r.exhausted, r.position), (None, true, after));
    // A position strictly after a row skips exactly that row.
    let first = seek(&db, RowFilter::Seat, None, 1).position.unwrap();
    assert_eq!(
        seek(&db, RowFilter::Seat, Some(first), 1)
            .row
            .unwrap()
            .1
            .message,
        msg("t", 5)
    );
    // The publication bound is inside the seek.
    let low = |after| {
        seek_pending(
            &db,
            "34750f98bd59fcfc946da45aaabe933b",
            &s(),
            RowFilter::Seat,
            after,
            (5 + DBASE) as u64,
            1,
        )
        .unwrap()
    };
    let first = low(None);
    assert_eq!(
        first.row.unwrap().1.message,
        msg("t", 5),
        "the p0 row (decision 6) is not yet published"
    );
    let second = low(first.position);
    assert_eq!(second.row.unwrap().1.message, msg("t", 3));
    assert!(low(second.position).exhausted);
}

#[test]
fn thread_scoped_and_binding_filtered_seeks() {
    let mut db = db();
    add_thread(&db, "t2");
    let b = enroll_binding(&db, "sess-B", None);
    publish(
        &mut db,
        &[
            Row {
                thread: "t2",
                n: 1,
                p: 1,
                d: 1,
            },
            Row {
                thread: "t",
                n: 3,
                p: 1,
                d: 2,
            },
            Row {
                thread: "t",
                n: 4,
                p: 1,
                d: 3,
            },
        ],
    );
    assert_eq!(
        drain(&db, RowFilter::Thread(&t("t"))),
        [msg("t", 3), msg("t", 4)]
    );
    let r = seek(&db, RowFilter::Thread(&t("t")), None, 10);
    assert_eq!(
        r.visited, 1,
        "the thread twin never visits t2's earlier row"
    );
    assert_eq!(drain(&db, RowFilter::Thread(&t("t2"))), [msg("t2", 1)]);
    // The binding's cursor is past t3: the row left the binding's view, so the seek returns t4 at once.
    read_cursors::advance(&db, &b, &t("t"), 3).unwrap();
    let after_t2 = seek(&db, RowFilter::Binding(&b), None, 1).position;
    let r = seek(&db, RowFilter::Binding(&b), after_t2, 10);
    assert_eq!(r.row.unwrap().1.message, msg("t", 4));
    assert_eq!(r.visited, 1, "a row the cursor passed is not in the view");
    assert_eq!(
        drain(&db, RowFilter::Binding(&b)),
        [msg("t2", 1), msg("t", 4)]
    );
}

#[test]
fn thread_binding_seek_stops_below_the_first_unread_receipt_row() {
    let mut db = db();
    let b = enroll_binding(&db, "sess-B", None);
    publish(
        &mut db,
        &[
            Row {
                thread: "t",
                n: 3,
                p: 1,
                d: 1,
            },
            Row {
                thread: "t",
                n: 6,
                p: 1,
                d: 2,
            },
        ],
    );
    // An ordinary message 5 owing the seat a receipt: the display walk cannot pass it.
    db.execute_batch(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,actor_seat_id) VALUES('ord-5','34750f98bd59fcfc946da45aaabe933b','t',5,'ordinary',5,'b',0,'a');
         INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES('ord-5','t','s','pending',100);
         INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('prep-ord-5','34750f98bd59fcfc946da45aaabe933b','actor','prep-ord-5',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');
         INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('prep-ord-5','t','s',1,100,1);
         INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('prep-ord-5','ord-5','34750f98bd59fcfc946da45aaabe933b','t',5,0,5,0,1,0);",
    )
    .unwrap();
    let (tid, seat) = (t("t"), s());
    let stop = read_cursors::first_ordinary_walk_stop(&db, &b, &seat, &tid)
        .unwrap()
        .unwrap();
    assert_eq!(stop, 5);
    let filter = RowFilter::ThreadBinding {
        binding: &b,
        thread: &tid,
        stop: stop as i64,
    };
    assert_eq!(drain(&db, filter), [msg("t", 3)], "t6 is above the stop");
    let r = seek(&db, filter, seek(&db, filter, None, 1).position, 10);
    assert_eq!(
        (r.row, r.visited, r.exhausted),
        (None, 1, true),
        "t6 costs a visit and is rejected"
    );
    assert_eq!(
        drain(&db, RowFilter::Binding(&b)),
        [msg("t", 3), msg("t", 6)],
        "the unscoped binding filter has no stop"
    );
}

#[test]
fn private_presentation_filter_matches_today() {
    let mut db = db();
    let b = enroll_binding(&db, "sess-B", None);
    publish(
        &mut db,
        &[
            Row {
                thread: "t",
                n: 3,
                p: 1,
                d: 1,
            },
            Row {
                thread: "t",
                n: 4,
                p: 1,
                d: 2,
            },
            Row {
                thread: "t",
                n: 5,
                p: 0,
                d: 3,
            },
        ],
    );
    read_cursors::advance(&db, &b, &t("t"), 3).unwrap();
    let expected: Vec<MessageId> = [msg("t", 5), msg("t", 3), msg("t", 4)]
        .into_iter()
        .filter(|m| {
            db.query_row(
                crate::store::queries::PRIVATE_LAZY_ELIGIBILITY_SQL,
                params![
                    "s",
                    m.as_str(),
                    b.as_str(),
                    "34750f98bd59fcfc946da45aaabe933b",
                    DH as i64
                ],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(
        expected,
        [msg("t", 5), msg("t", 4)],
        "today's statement rejects the row behind the cursor"
    );
    assert_eq!(drain(&db, RowFilter::PrivatePresentation(&b)), expected);
}

#[test]
fn published_rows_get_decision_seq_exactly_once() {
    let mut db = db();
    publish(
        &mut db,
        &[Row {
            thread: "t",
            n: 3,
            p: 1,
            d: 7,
        }],
    );
    let value = |db: &Connection| -> Option<i64> {
        db.query_row("SELECT decision_seq FROM lazy_recipients", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(value(&db), Some(7 + DBASE), "the manifest's decision");
    let tx = db.transaction().unwrap();
    note_published(&tx, "prep-t-3").unwrap();
    tx.commit().unwrap();
    assert_eq!(
        value(&db),
        Some(7 + DBASE),
        "a second publication note changes nothing"
    );
    let err = db
        .execute("UPDATE lazy_recipients SET decision_seq=8", [])
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("delivery row publication is immutable"),
        "{err}"
    );
    assert_eq!(value(&db), Some(7 + DBASE));
}

/// p1 seq 3 (decision 2) then p0 seq 4 (decision 1) in thread `t`: the p0 row displays first.
fn out_of_order(db: &mut Connection) {
    publish(
        db,
        &[
            Row {
                thread: "t",
                n: 3,
                p: 1,
                d: 2,
            },
            Row {
                thread: "t",
                n: 4,
                p: 0,
                d: 1,
            },
        ],
    );
}

#[test]
fn out_of_order_display_is_not_reoffered() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let sub = enroll_binding(&db, "sess-A", Some(&a));
    let body = enroll_binding(&db, "sess-C", None);
    out_of_order(&mut db);
    assert_eq!(unread(&db, &a), [msg("t", 3), msg("t", 4)]);
    // Top-level completion.
    complete(&mut db, &a, &[msg("t", 4)]);
    assert_eq!(
        unread(&db, &a),
        [msg("t", 3)],
        "seq 4 is not re-offered, seq 3 still is"
    );
    assert_eq!(marks(&db, &a), ["msg-t-4"]);
    assert_eq!(read_cursors::cursor_for(&db, &a, &t("t")).unwrap(), Some(2));
    assert_eq!(drain(&db, RowFilter::Binding(&a)), [msg("t", 3)]);
    assert_eq!(
        seat_lazy_pending(&db, &s(), 100).unwrap().0,
        2,
        "the seat view keeps rows other bindings still owe"
    );
    // The cursor-only `AdvanceCursors` path (subagent streams, body reads) is this one function.
    for (binding, _what) in [(&sub, "subagent stream"), (&body, "body read")] {
        let tx = db.transaction().unwrap();
        let (reached, _) =
            read_cursors::advance_through_displayed_with_change(&tx, binding, &t("t"), &[4])
                .unwrap();
        tx.commit().unwrap();
        assert_eq!(reached, 2);
        assert_eq!(unread(&db, binding), [msg("t", 3)]);
        assert_eq!(marks(&db, binding), ["msg-t-4"]);
    }
    // Displaying seq 3 passes the cursor over 4 and prunes the marks.
    complete(&mut db, &a, &[msg("t", 3)]);
    assert_eq!(read_cursors::cursor_for(&db, &a, &t("t")).unwrap(), Some(4));
    assert!(marks(&db, &a).is_empty());
    assert!(unread(&db, &a).is_empty());
    assert_eq!(
        unread(&db, &sub),
        [msg("t", 3)],
        "another binding's marks and cursor are its own"
    );
}

#[test]
fn resumed_binding_inherits_marks() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let lineage = enroll_binding(&db, "sess-L", None);
    out_of_order(&mut db);
    complete(&mut db, &a, &[msg("t", 4)]);
    assert!(marks(&db, &lineage).is_empty());
    assert_eq!(
        unread(&db, &lineage),
        [msg("t", 3), msg("t", 4)],
        "lineage binding has not displayed it"
    );
    // `copy_lineage`: the successor takes the predecessor's cursors and marks.
    read_cursors::copy_lineage(&db, &a, &lineage).unwrap();
    assert_eq!(marks(&db, &lineage), ["msg-t-4"]);
    assert_eq!(unread(&db, &lineage), [msg("t", 3)]);
    // `init_for_new_binding`: a binding enrolled now inherits the seat-maximum cursor and the marks
    // of the binding that holds it, above that cursor.
    let fresh = enroll_binding(&db, "sess-F", None);
    assert_eq!(marks(&db, &fresh), ["msg-t-4"]);
    assert_eq!(unread(&db, &fresh), [msg("t", 3)]);
}
