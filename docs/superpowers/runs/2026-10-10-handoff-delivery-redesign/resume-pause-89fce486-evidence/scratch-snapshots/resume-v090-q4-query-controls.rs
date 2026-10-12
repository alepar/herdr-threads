//! Actual Q4 SQL/work witnesses; setup is outside every measured region.
use super::{bind_query_agent, budget, fixture};
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness},
    ids::{
        BindingId, ExecutionId, HostTargetId, InvitationId, MessageId, NativeSessionId, SeatId,
        ThreadId,
    },
};
use crate::store::{invitation_content, lazy_delivery, queries};
use rusqlite::{Connection, StatementStatus, params, params_from_iter, types::Value};
use sha2::{Digest, Sha256};

fn text(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}

fn seeded(
    retained: i64,
) -> (
    crate::store::connection::StoreContext,
    Connection,
    BindingId,
) {
    seeded_body(retained, false, 100)
}

fn seeded_body(
    retained: i64,
    long: bool,
    candidates: i64,
) -> (
    crate::store::connection::StoreContext,
    Connection,
    BindingId,
) {
    let (store, mut db) = fixture();
    bind_query_agent(&db);
    let binding = BindingId::new(
        db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE native_session='session'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    );
    let tx = db.transaction().unwrap();
    // Complete migrated schema, FK/trigger checks enabled. Canonical-shaped
    // raw rows isolate reader SQL; this does not test public invitation writers.
    for n in 0..candidates {
        let thread = format!("q4-t{n}");
        let sender = format!("a-q4-{n}");
        tx.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,'i','resolved','native',1,0)", [&sender]).unwrap();
        tx.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES (?1,'i','q4','q4',0,0,100001)", [&thread]).unwrap();
        if !(50..75).contains(&n) {
            tx.execute("INSERT INTO direct_threads(instance_id,seat_a,seat_b,thread_id) VALUES ('i',?1,'s',?2)", params![sender,thread]).unwrap();
        }
        let voluntary = if n >= 75 { "left" } else { "invited" };
        tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,'s',10000,'invited',?2)", params![thread,voluntary]).unwrap();
        tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES (?1,?2,'s',10000,'pending',0,1000000,300,300,'lazy',?3)", params![format!("q4-inv{n}"),thread,sender]).unwrap();
        let prep = format!("prep-q4-{n}");
        let message = format!("msg-q4-{n}");
        tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES (?1,'i','actor',?1,zeroblob(32),?2,0,0,0,0,0,0,0,'building','lazy')", params![prep,thread]).unwrap();
        tx.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES (?1,?2,?3,'s')", params![prep,message,thread]).unwrap();
        let decision = if (25..50).contains(&n) {
            100000 + n
        } else {
            2000000 + n
        };
        let body = if long && n == 0 {
            "☃".repeat(21845)
        } else {
            "q4-body".into()
        };
        tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at,delivery_mode) VALUES (?1,'i',?2,100000,'ordinary',?3,?5,?4,0,'lazy')", params![message,thread,sender,decision,body]).unwrap();
        tx.execute(
            "UPDATE send_preparations SET status='sealed' WHERE id=?1",
            [&prep],
        )
        .unwrap();
        tx.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,?3,?4,0,100000,0,1,0)", params![prep,message,thread,decision]).unwrap();
    }
    for n in 0..retained {
        let sender = format!("a-noise-{n}");
        let thread = format!("noise-t{n}");
        tx.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,'i','resolved','native',1,0)", [&sender]).unwrap();
        tx.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','noise','noise',0,0)", [&thread]).unwrap();
        tx.execute("INSERT INTO direct_threads(instance_id,seat_a,seat_b,thread_id) VALUES ('i',?1,'s',?2)", params![sender,thread]).unwrap();
        tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,'s',1,'invited','invited')", [&thread]).unwrap();
        tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES (?1,?2,'s',1,'pending',0,?3,300,300)", params![format!("noise-inv{n}"),thread,n+1]).unwrap();
        // Exact current episode is fixed at10000 in both populations. Older
        // retained episodes and event history grow without moving that boundary.
        tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,retired_at) VALUES (?1,'q4-t0','s',?2,'recipient_retired',0,?2,300,300,1)", params![format!("old-inv{n}"),n+1]).unwrap();
        tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_key,event_json,decision_seq,decision_at) VALUES (?1,'i','q4-t0',?2,'info',?1,'{}',?3,0)", params![format!("noise-event{n}"),n+1,500000+n]).unwrap();
    }
    for (key, sequence, decision) in [
        ("invite:q4-inv0", 90000, 1000001),
        ("operator_invite:q4-inv0", 90001, 1000002),
        ("service_invite:q4-inv0:ordinary", 90002, 1000003),
    ] {
        tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_key,event_json,source_invitation_id,decision_seq,decision_at) VALUES (?1,'i','q4-t0',?2,'info',?1,'{}','q4-inv0',?3,0)", params![key,sequence,decision]).unwrap();
    }
    tx.commit().unwrap();
    (store, db, binding)
}

fn indexed(
    db: &Connection,
    sql: &str,
    values: Vec<Value>,
    aliases: &[&str],
) -> (Vec<Vec<Value>>, i32) {
    let plan = db
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(params_from_iter(&values), |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    // SELECT EXISTS legitimately has one constant outer row. It is not a
    // retained relation scan and must not be confused with a source visit.
    assert!(
        !plan.iter().any(
            |line| (line.starts_with("SCAN ") && line != "SCAN CONSTANT ROW")
                || line.contains("TEMP B-TREE")
        ),
        "{plan:?}"
    );
    if values.iter().any(|value| matches!(value, Value::Text(id) if id == "msg-q4-0" || id.starts_with("invite:q4-inv0") || id.starts_with("operator_invite:q4-inv0") || id.starts_with("service_invite:q4-inv0"))) {
        eprintln!("Q4 actualSQL plan={plan:?}");
    }
    let searches = plan
        .iter()
        .filter(|line| line.starts_with("SEARCH "))
        .collect::<Vec<_>>();
    assert_eq!(searches.len(), aliases.len(), "{plan:?}");
    for alias in aliases {
        let want = aliases.iter().filter(|other| *other == alias).count();
        assert_eq!(
            searches
                .iter()
                .filter(|line| line.starts_with(&format!("SEARCH {alias} "))
                    && line.contains("USING ")
                    && (line.contains("INDEX") || line.contains("PRIMARY KEY")))
                .count(),
            want,
            "alias={alias} {plan:?}"
        );
    }
    let mut statement = db.prepare(sql).unwrap();
    let columns = statement.column_count();
    let rows = statement
        .query_map(params_from_iter(&values), |row| {
            (0..columns).map(|column| row.get(column)).collect()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        statement.get_status(StatementStatus::FullscanStep),
        0,
        "{plan:?}"
    );
    let vm = statement.get_status(StatementStatus::VmStep);
    // Failures show the complete plan. VM invariance, not wall time/I/O,
    // is asserted below; callers print the query provenance and measured work.
    (rows, vm)
}

fn claim() -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("session"),
        execution: ExecutionId::new("exec"),
        target: HostTargetId::new("w:p1"),
        agent_id: None,
        liveness: None,
    }
}

/// Kills hidden history/seat-wide searches and redundant messages/membership
/// accesses, while checking literal current/pre-episode/nonpair/terminal results.
#[test]
fn q4_episode_private_and_reconciliation_points_have_constant_sqlite_work() {
    let mut measurements = Vec::new();
    for retained in [128, 8192] {
        let (_store, mut db, binding) = seeded(retained);
        let tx = db.transaction().unwrap();
        let mut work = Vec::new();
        for n in 0..100 {
            let id = format!("msg-q4-{n}");
            let want = vec![vec![Value::Integer(if n < 25 { 1 } else { 0 })]];
            let private = indexed(
                &tx,
                queries::PRIVATE_LAZY_ELIGIBILITY_SQL,
                vec![
                    text("s"),
                    text(&id),
                    text(binding.as_str()),
                    text("i"),
                    Value::Integer(2100000),
                ],
                &["r", "sm", "m", "b", "lc", "lc", "lm", "ld", "ls", "li"],
            );
            assert_eq!(private.0, want, "private candidate={n}");
            let canonical = indexed(
                &tx,
                invitation_content::OFFER_RECONCILIATION_MESSAGE_SQL,
                vec![text(&id), text("i")],
                &["messages"],
            );
            assert_eq!(canonical.0.len(), 1);
            let row = &canonical.0[0];
            assert_eq!(row[0], text("lazy"));
            let recon = indexed(
                &tx,
                invitation_content::OFFER_RECONCILIATION_LAZY_SQL,
                vec![
                    text("s"),
                    text(&id),
                    text(binding.as_str()),
                    text("i"),
                    row[1].clone(),
                    row[2].clone(),
                    row[3].clone(),
                ],
                &["r", "sm", "b", "lc", "lc", "lm", "ld", "ls", "li"],
            );
            assert_eq!(recon.0, want, "recon candidate={n}");
            let continuation = indexed(
                &tx,
                queries::LAZY_CONTINUATION_SCOPE_SQL,
                vec![text("s"), text(&id), text(binding.as_str()), text("i")],
                &["m", "lc", "lm", "ld", "ls", "li"],
            );
            assert_eq!(continuation.0, want, "continued scope candidate={n}");
            work.push((private.1, canonical.1, recon.1, continuation.1));
        }
        eprintln!(
            "Q4 actual pointSQL SQLite={} noise={retained} same100 VM={work:?} lazy_source_SHA256={:x} query_source_SHA256={:x} recon_source_SHA256={:x}",
            tx.query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))
                .unwrap(),
            Sha256::digest(include_str!("../../src/store/lazy_delivery.rs").as_bytes()),
            Sha256::digest(include_str!("../../src/store/queries.rs").as_bytes()),
            Sha256::digest(include_str!("../../src/store/invitation_content.rs").as_bytes())
        );
        measurements.push(work);
        tx.rollback().unwrap();
    }
    assert_eq!(
        measurements[0], measurements[1],
        "identical keys/branches must not walk unrelated pair/invitation/event history"
    );
}

/// Kills loss of same-snapshot canonical identity in optimized reconciliation,
/// and scope predicates that admit a retired seat through its pending invitation.
#[test]
fn q4_scalar_agreement_and_strict_retirement_use_exact_scope() {
    let (_store, mut db, binding) = seeded(128);
    let tx = db.transaction().unwrap();
    let canonical: (String, String, i64, i64) = tx
        .query_row(
            invitation_content::OFFER_RECONCILIATION_MESSAGE_SQL,
            params!["msg-q4-0", "i"],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(canonical, ("lazy".into(), "q4-t0".into(), 100000, 2000000));
    let values = vec![
        text("s"),
        text("msg-q4-0"),
        text(binding.as_str()),
        text("i"),
        text(&canonical.1),
        Value::Integer(canonical.2),
        Value::Integer(canonical.3),
    ];
    let aliases = ["r", "sm", "b", "lc", "lc", "lm", "ld", "ls", "li"];
    assert_eq!(
        indexed(
            &tx,
            invitation_content::OFFER_RECONCILIATION_LAZY_SQL,
            values.clone(),
            &aliases
        )
        .0,
        [vec![Value::Integer(1)]]
    );
    for (index, bad) in [
        (0, text("missing-seat")),
        (1, text("missing-message")),
        (2, text("missing-binding")),
        (3, text("other-instance")),
        (4, text("q4-t1")),
        (6, Value::Integer(2000001)),
    ] {
        let mut perturbed = values.clone();
        perturbed[index] = bad;
        assert!(
            indexed(
                &tx,
                invitation_content::OFFER_RECONCILIATION_LAZY_SQL,
                perturbed,
                &aliases
            )
            .0
            .is_empty(),
            "identity field={index} cannot supply Some(false) terminal proof"
        );
    }
    // Sequence is trusted from the preceding canonical point, not a public
    // scalar API. The second query intentionally performs no second lookup.
    tx.execute(
        "UPDATE seats SET state='retired',retired_at=1 WHERE id='s'",
        [],
    )
    .unwrap();
    let episode = indexed(
        &tx,
        lazy_delivery::ACTIVE_DIRECT_EPISODE_SQL,
        vec![text("q4-t0"), text("s"), text("i")],
        &["lm", "ld", "ls", "li"],
    );
    assert!(
        episode.0.is_empty(),
        "a canonically retired seat has no current invited episode"
    );
    let scope = indexed(
        &tx,
        queries::LAZY_CONTINUATION_SCOPE_SQL,
        vec![
            text("s"),
            text("msg-q4-0"),
            text(binding.as_str()),
            text("i"),
        ],
        &["m", "lc", "lm", "ld", "ls", "li"],
    );
    assert_eq!(scope.0, [vec![Value::Integer(0)]]);
    tx.rollback().unwrap();
}

/// Kills event-history/source_invitation scans and choosing a later producer's
/// event instead of the earliest valid one among the three fixed unique keys.
#[test]
fn q4_three_event_keys_and_actual_floor_have_constant_sqlite_work() {
    use crate::test_support::isolation::{CostCounter, count_vm_units};
    let mut measured = Vec::new();
    for retained in [128, 8192] {
        let (_store, mut db, _binding) = seeded(retained);
        let tx = db.transaction().unwrap();
        let mut vm = Vec::new();
        for (key, seq, decision) in [
            ("invite:q4-inv0", 90000, 1000001),
            ("operator_invite:q4-inv0", 90001, 1000002),
            ("service_invite:q4-inv0:ordinary", 90002, 1000003),
        ] {
            let event = indexed(
                &tx,
                lazy_delivery::DIRECT_EPISODE_EVENT_SQL,
                vec![text(key)],
                &["messages"],
            );
            assert_eq!(
                event.0,
                [vec![
                    text("i"),
                    text("q4-t0"),
                    text("info"),
                    text("q4-inv0"),
                    Value::Integer(decision),
                    Value::Integer(seq)
                ]]
            );
            vm.push(event.1);
        }
        let counter = CostCounter::default();
        let floor = count_vm_units(&tx, &counter, || {
            lazy_delivery::direct_acceptance_floor(
                &tx,
                "i",
                &SeatId::new("s"),
                &ThreadId::new("q4-t0"),
                &InvitationId::new("q4-inv0"),
            )
            .unwrap()
        });
        assert_eq!(floor, Some(90000));
        measured.push((vm, counter.units()));
        eprintln!(
            "Q4 eventkeys noise={retained} actual_floor_VM_units={}",
            counter.units()
        );
        tx.rollback().unwrap();
    }
    assert_eq!(measured[0], measured[1]);
}

/// Kills omitted/doubled new scope charging and charging only rendered rows.
/// It executes actual private raw discovery and actual exact-ID due hydration.
#[test]
fn q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers() {
    use crate::test_support::isolation::{CostCounter, count_vm_units};
    let mut measured = Vec::new();
    for retained in [128, 8192] {
        let (store, mut db, binding) = seeded(retained);
        let tx = db.transaction().unwrap();
        let counter = CostCounter::default();
        let fresh = count_vm_units(&tx, &counter, || {
            queries::ordinary_presentation_page(&tx, &store, "i", &claim(), None, &budget())
                .unwrap()
        });
        assert_eq!(fresh.visited, 100);
        assert_eq!(fresh.lazy_scope_probes, 300);
        assert_eq!(
            fresh.page.items.len(),
            25,
            "three rejected groups consume source work without output"
        );
        assert_eq!(fresh.continuation_probes, 0);
        assert_eq!(fresh.retired_body, None);
        let mut due = crate::ports::PresentationWork::default();
        let due_counter = CostCounter::default();
        let item = count_vm_units(&tx, &due_counter, || {
            queries::ordinary_presentation_message_by_id(
                &tx,
                "i",
                &SeatId::new("s"),
                &binding,
                &MessageId::new("msg-q4-0"),
                (2100000, true),
                &mut due,
            )
            .unwrap()
        });
        assert!(
            matches!(item,Some(crate::protocol::results::InboxBatchV2Item::LazyMessage { ref message,.. }) if message.as_str()=="msg-q4-0")
        );
        assert_eq!(due.ordinary_due_key_probes, 14);
        assert_eq!(
            due.ordinary_lazy_scope_probes, 0,
            "due mode charges its own counter only"
        );
        assert_eq!(due.offer_reconciliation_probes, 0);
        measured.push((counter.units(), due_counter.units()));
        eprintln!(
            "Q4 noise={retained} actual_fresh_VM_units={} actual_due_VM_units={}",
            counter.units(),
            due_counter.units()
        );
        tx.rollback().unwrap();
    }
    assert_eq!(measured[0], measured[1]);
}

// Connection-local statement observer; no process-global failpoint or hook.
struct ScopeTrace<'a> {
    db: &'a Connection,
    counts: Box<[std::cell::Cell<usize>; 2]>,
}
impl<'a> ScopeTrace<'a> {
    fn install(db: &'a Connection) -> Self {
        unsafe extern "C" fn record(
            _: u32,
            data: *mut std::ffi::c_void,
            statement: *mut std::ffi::c_void,
            _: *mut std::ffi::c_void,
        ) -> i32 {
            // SAFETY: the guard retains its boxed cells until after unregister;
            // this connection and its callback execute synchronously here.
            let cells = unsafe { &*data.cast::<[std::cell::Cell<usize>; 2]>() };
            let raw = unsafe { rusqlite::ffi::sqlite3_sql(statement.cast()) };
            if !raw.is_null() {
                let sql = unsafe { std::ffi::CStr::from_ptr(raw) }.to_bytes();
                for (i, wanted) in [
                    queries::PRIVATE_LAZY_ELIGIBILITY_SQL,
                    queries::LAZY_CONTINUATION_SCOPE_SQL,
                ]
                .iter()
                .enumerate()
                {
                    if sql == wanted.as_bytes() {
                        cells[i].set(cells[i].get() + 1);
                    }
                }
            }
            0
        }
        let observer = Self {
            db,
            counts: Box::new([std::cell::Cell::new(0), std::cell::Cell::new(0)]),
        };
        // SAFETY: boxed callback state has a stable address; Drop unregisters
        // before the box is freed and the borrowed connection must outlive us.
        assert_eq!(
            unsafe {
                rusqlite::ffi::sqlite3_trace_v2(
                    db.handle(),
                    rusqlite::ffi::SQLITE_TRACE_STMT,
                    Some(record),
                    observer.counts.as_ref() as *const _ as *mut std::ffi::c_void,
                )
            },
            rusqlite::ffi::SQLITE_OK
        );
        observer
    }
    fn counts(&self) -> [usize; 2] {
        [self.counts[0].get(), self.counts[1].get()]
    }
}
impl Drop for ScopeTrace<'_> {
    fn drop(&mut self) {
        // SAFETY: the connection is still borrowed and callback state still live.
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(self.db.handle(), 0, None, std::ptr::null_mut());
        }
    }
}

/// Kills duplicate SQL during sizing or continued-tail handling, scope checks
/// that erase a captured tail after cursor movement, and uncharged retirement.
#[test]
fn q4_actual_continued_tail_checks_scope_once_and_charges_retirement() {
    use crate::protocol::{pagination::InboxBatchV2CursorState, results::InboxBatchV2Item};
    for lose_scope in [false, true] {
        let (store, mut db, binding) = seeded_body(128, true, 1);
        let tx = db.transaction().unwrap();
        let trace = ScopeTrace::install(&tx);
        let first =
            queries::ordinary_presentation_page(&tx, &store, "i", &claim(), None, &budget())
                .unwrap();
        assert_eq!(trace.counts(), [1, 0]);
        drop(trace);
        assert_eq!((first.visited, first.lazy_scope_probes), (1, 3));
        let cursor = first
            .page
            .next_cursor
            .expect("a real long body pins a tail");
        let captured = InboxBatchV2CursorState::decode(&cursor).unwrap();
        let body = captured.body.expect("captured body");
        assert_eq!(body.message.as_str(), "msg-q4-0");
        assert!(body.offset > 0 && body.offset < body.body_len);
        if lose_scope {
            tx.execute("UPDATE memberships SET state='left',voluntary_state='left' WHERE thread_id='q4-t0' AND seat_id='s'",[]).unwrap();
        } else {
            // Explicit fixture display progress: the tail keeps its captured
            // identity/offset, even though a fresh unread predicate is now false.
            crate::store::read_cursors::advance(&tx, &binding, &ThreadId::new("q4-t0"), 100000)
                .unwrap();
        }
        let trace = ScopeTrace::install(&tx);
        let continued = queries::ordinary_presentation_page(
            &tx,
            &store,
            "i",
            &claim(),
            Some(cursor),
            &budget(),
        )
        .unwrap();
        assert_eq!(
            trace.counts(),
            [0, 1],
            "only one actual continuation SQL, no fresh eligibility or sizing requery"
        );
        drop(trace);
        assert_eq!((continued.visited, continued.lazy_scope_probes), (1, 3));
        assert_eq!(
            continued.continuation_probes, 0,
            "ordinary continuation has independent accounting"
        );
        if lose_scope {
            assert_eq!(continued.retired_body, Some(MessageId::new("msg-q4-0")));
            assert!(continued.page.items.is_empty());
        } else {
            assert_eq!(continued.retired_body, None);
            assert!(
                matches!(continued.page.items.as_slice(),[InboxBatchV2Item::LazyMessage{message,body_start,body_end,body_len,..}]
                if message.as_str()=="msg-q4-0" && *body_start==body.offset && *body_end==body.body_len && *body_len==body.body_len)
            );
        }
        tx.rollback().unwrap();
    }
}
