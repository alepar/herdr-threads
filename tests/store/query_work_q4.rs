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
    seeded_body(retained, false, 100, true)
}

fn seeded_body(
    retained: i64,
    long: bool,
    candidates: i64,
    same_seat_recipients: bool,
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
        tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,?2,1,'joined','joined')",params![thread,sender]).unwrap();
        tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES (?1,?2,'s',1,'pending',0,?3,300,300)", params![format!("noise-inv{n}"),thread,n+1]).unwrap();
        // Same-seat historical identities defeat a seat-prefix binding search.
        // Both cursor-key directions have distractors: the selected binding on
        // unrelated threads, and unrelated bindings on the selected thread.
        let other_binding = format!("noise-binding-{n}");
        tx.execute("INSERT INTO occupant_bindings(binding_id,seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,ended_at,liveness,disconnected_at,disconnect_reason,terminal_id,incarnation) VALUES (?1,'s',?2,1,'w:p1','host',1,'codex',?3,?4,'cooperative_top_level',0,0,1,'disconnected',1,'ended','term','inc')", params![other_binding,n+2,format!("noise-session-{n}"),format!("noise-exec-{n}")]).unwrap();
        for (cursor_binding, cursor_thread) in [
            (binding.as_str(), thread.as_str()),
            (other_binding.as_str(), "q4-t0"),
            (other_binding.as_str(), thread.as_str()),
        ] {
            tx.execute(
                "INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES (?1,?2,1)",
                params![cursor_binding, cursor_thread],
            )
            .unwrap();
        }
        // Published keys grow, not only preparations. In the point fixture,
        // selected-seat distractors are after the fixed first100 ordinals. Raw
        // page/tail fixtures use the other seat only, keeping their captured
        // selected-seat high-water and exhaustion branch identical.
        let prep = format!("prep-noise-{n}");
        let message = format!("msg-noise-{n}");
        tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES (?1,'i','actor',?1,zeroblob(32),?2,0,0,0,0,0,0,0,'building','lazy')",params![prep,thread]).unwrap();
        tx.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES (?1,?2,?3,?4)",params![prep,message,thread,sender]).unwrap();
        if same_seat_recipients {
            tx.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES (?1,?2,?3,'s')",params![prep,message,thread]).unwrap();
        }
        tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at,delivery_mode) VALUES (?1,'i',?2,1,'ordinary',?3,'noise',?4,0,'lazy')",params![message,thread,sender,600000+n]).unwrap();
        tx.execute(
            "UPDATE send_preparations SET status='sealed' WHERE id=?1",
            [&prep],
        )
        .unwrap();
        tx.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,?3,?4,0,1,0,?5,0)",params![prep,message,thread,600000+n,if same_seat_recipients {2} else {1}]).unwrap();
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
    // Cardinality bookends run outside all measured regions and prove that
    // every relation accessed by the composed points actually grows.
    for (table, expected) in [
        ("occupant_bindings", retained + 1),
        ("binding_cursors", retained * 3),
        (
            "lazy_recipients",
            candidates + retained * if same_seat_recipients { 2 } else { 1 },
        ),
        ("send_manifests", candidates + retained),
        ("memberships", candidates + retained * 2),
        ("seats", candidates + retained + 1),
        ("threads", candidates + retained + 1),
        (
            "direct_threads",
            candidates - (50..75).filter(|n| *n < candidates).count() as i64 + retained,
        ),
        ("invitations", candidates + retained * 2),
        ("messages", candidates + retained * 2 + 3),
    ] {
        let count: i64 = tx
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, expected, "fixture cardinality {table}");
    }
    let same_binding_cursors: i64 = tx
        .query_row(
            "SELECT count(*) FROM binding_cursors WHERE binding_id=?1",
            [binding.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(same_binding_cursors, retained);
    let selected_recipients: i64 = tx
        .query_row(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        selected_recipients,
        candidates + if same_seat_recipients { retained } else { 0 }
    );
    let selected_keys = tx
        .prepare(
            "SELECT message_id FROM lazy_recipients WHERE seat_id='s' ORDER BY ordinal LIMIT ?1",
        )
        .unwrap()
        .query_map([candidates], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        selected_keys,
        (0..candidates)
            .map(|n| format!("msg-q4-{n}"))
            .collect::<Vec<_>>()
    );
    let selected_cursors: i64 = tx
        .query_row(
            "SELECT count(*) FROM binding_cursors WHERE binding_id=?1 AND thread_id GLOB 'q4-t*'",
            [binding.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(selected_cursors, 0, "candidate threads remain cursorless");
    if !same_seat_recipients {
        let high: i64 = tx
            .query_row(
                "SELECT max(ordinal) FROM lazy_recipients WHERE seat_id='s'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            high, candidates,
            "raw/tail selected high-water is independent of noise"
        );
    }
    tx.commit().unwrap();
    (store, db, binding)
}

// These are schema UNIQUE/PRIMARY KEY column sets, not copied query
// predicates. Autoindex numbering and join order are deliberately irrelevant.
// A SEARCH on an indexed prefix/range does not certify a point lookup.
fn unique_point(line: &str, alias: &str, event: bool) -> bool {
    if !line.starts_with(&format!("SEARCH {alias} "))
        || !line.contains("USING ")
        || !(line.contains("INDEX") || line.contains("PRIMARY KEY"))
    {
        return false;
    }
    let Some((_, suffix)) = line.rsplit_once(" (") else {
        return false;
    };
    let Some(constraints) = suffix.strip_suffix(')') else {
        return false;
    };
    let terms = constraints.split(" AND ").collect::<Vec<_>>();
    let keys: &[&[&str]] = match alias {
        "r" => &[
            &["message_id=?", "seat_id=?"],
            &["preparation_id=?", "seat_id=?"],
        ],
        "sm" => &[
            &["preparation_id=?"],
            &["message_id=?"],
            &["instance_id=?", "decision_seq=?"],
        ],
        "b" => &[&["binding_id=?"]],
        "lc" => &[&["binding_id=?", "thread_id=?"]],
        "lm" => &[&["thread_id=?", "seat_id=?"]],
        "ld" => &[&["thread_id=?"]],
        "ls" => &[&["id=?"]],
        "li" => &[&["thread_id=?", "seat_id=?", "episode=?"]],
        "m" => &[&["id=?"]],
        "messages" if event => &[&["event_key=?"]],
        "messages" => &[&["id=?"]],
        _ => panic!("unmapped production SQL alias {alias}"),
    };
    keys.iter()
        .any(|key| key.iter().all(|term| terms.contains(term)))
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
        let actual = searches
            .iter()
            .filter(|line| line.starts_with(&format!("SEARCH {alias} ")))
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), want, "alias={alias} {plan:?}");
        for line in actual {
            assert!(
                unique_point(line, alias, sql == lazy_delivery::DIRECT_EPISODE_EVENT_SQL),
                "alias={alias} must constrain a complete unique equality key: {plan:?}"
            );
        }
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
        "identical keys/branches must not walk unrelated binding/cursor/recipient/manifest/pair/invitation/event keys"
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
        let (store, mut db, binding) = seeded_body(retained, false, 100, false);
        let tx = db.transaction().unwrap();
        let counter = CostCounter::default();
        // Fresh capture starts at ordinal0. The real source rotation can
        // reserve two warning slots at visit99 even when warning exhaustion
        // is already captured. Follow its actual returned cursor to inspect
        // all100 fixed raw candidates; do not invent an ordinal1 start, skip
        // a candidate, raise the per-page cap, or reinterpret the cursor.
        let trace = ScopeTrace::install(&tx);
        let (visited, scope, items, pages) = count_vm_units(&tx, &counter, || {
            let mut cursor = None;
            let mut visited = 0;
            let mut scope = 0;
            let mut items = Vec::new();
            let mut pages = 0;
            loop {
                let page = queries::ordinary_presentation_page(
                    &tx,
                    &store,
                    "i",
                    &claim(),
                    cursor.take(),
                    &budget(),
                )
                .unwrap();
                pages += 1;
                assert!(page.visited <= 100, "per-quantum physical cap");
                assert_eq!(page.lazy_scope_probes, page.visited * 3);
                assert_eq!(page.continuation_probes, 0);
                assert_eq!(page.retired_body, None);
                visited += page.visited;
                scope += page.lazy_scope_probes;
                items.extend(page.page.items);
                cursor = page.page.next_cursor;
                if let Some(raw) = &cursor {
                    let captured =
                        crate::protocol::pagination::InboxBatchV2CursorState::decode(raw).unwrap();
                    assert_eq!(
                        captured.lazy_after_ordinal, visited as u64,
                        "actual cursor tracks all inspected ordinals from0"
                    );
                    assert_eq!(captured.lazy_high_water_ordinal, 100);
                    assert!(captured.body.is_none(), "raw scan has no body tail");
                    assert!(
                        pages < 2,
                        "fixed100 slice must exhaust within two real quanta"
                    );
                } else {
                    break;
                }
            }
            (visited, scope, items, pages)
        });
        assert_eq!(
            trace.counts(),
            [100, 0],
            "one actual eligibility SQL per raw candidate"
        );
        drop(trace);
        assert_eq!(visited, 100);
        assert_eq!(scope, 300);
        assert_eq!(
            items.len(),
            25,
            "three rejected groups consume source work without output"
        );
        assert!(items.iter().enumerate().all(|(n, item)| matches!(item,
            crate::protocol::results::InboxBatchV2Item::LazyMessage {message,..}
            if message.as_str()==format!("msg-q4-{n}"))));
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
            "Q4 noise={retained} actual_raw100_VM_units={} actual_due_VM_units={} raw_quanta={pages}",
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
    counts: Box<[std::cell::Cell<usize>; 3]>,
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
            let cells = unsafe { &*data.cast::<[std::cell::Cell<usize>; 3]>() };
            let raw = unsafe { rusqlite::ffi::sqlite3_sql(statement.cast()) };
            if !raw.is_null() {
                let sql = unsafe { std::ffi::CStr::from_ptr(raw) }.to_bytes();
                if sql.starts_with(b"SELECT EXISTS(SELECT 1 FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id") {
                    cells[2].set(cells[2].get() + 1);
                }
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
            counts: Box::new(std::array::from_fn(|_| std::cell::Cell::new(0))),
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
    fn warning_audience_probes(&self) -> usize {
        self.counts[2].get()
    }
}

fn seeded_warning_history(retained: i64) -> (crate::store::connection::StoreContext, Connection) {
    let (store, mut db, _) = seeded_body(0, false, 2, false);
    let tx = db.transaction().unwrap();
    for episode in 1..=retained {
        tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq,left_seq) VALUES ('t','s',?1,1,2)", [episode]).unwrap();
    }
    // Every retained episode ended before the warning. No recipient projection
    // hides the negative frozen-audience lookup behind a positive point result.
    tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_seq,decision_at) VALUES ('q4-negative-warning','i','t',1,'warn','{}',10,0)", []).unwrap();
    tx.execute("INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('q4-negative-warning',10,'t',?1,'a-q4-0','invitation','negative-invitation')", [retained]).unwrap();
    tx.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_seq,decision_at) VALUES ('q4-origin-warning','i','q4-t0',100001,'warn','{}',2000002,0);
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('q4-origin-warning',2000002,'q4-t0',0,'s','invitation','q4-inv0');
        INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,episode,open_warning_id,opened_seq,invitation_originator_instance_id,invitation_originator_seat_id) VALUES ('invitation','q4-t0','q4-inv0','s',10000,'q4-origin-warning',2000002,'i','a-q4-0');").unwrap();
    let intervals: i64 = tx.query_row("SELECT count(*) FROM membership_intervals WHERE thread_id='t' AND seat_id='s' AND left_seq=2", [], |r| r.get(0)).unwrap();
    assert_eq!(
        intervals, retained,
        "retained negative-audience history grows"
    );
    let projected: i64 = tx
        .query_row("SELECT count(*) FROM warning_recipients", [], |r| r.get(0))
        .unwrap();
    assert_eq!(projected, 0, "warning audience remains unprojected");
    tx.commit().unwrap();
    (store, db)
}

fn public_warning_page(
    db: &Connection,
    store: &crate::store::connection::StoreContext,
    cursor: Option<String>,
    limit: u16,
) -> crate::protocol::pagination::Page<crate::protocol::results::InboxBatchV2Item> {
    let result = queries::inbox_v2::query(
        db,
        store,
        "i",
        &crate::protocol::commands::InboxQuery {
            seat: Some(SeatId::new("s")),
            claim: Some(claim()),
            page: crate::protocol::pagination::PageRequest {
                cursor,
                limit,
                max_bytes: 65536,
            },
        },
        &crate::protocol::output::OutputSpec::default(),
        &budget(),
    )
    .unwrap();
    let crate::protocol::results::CommandResult::InboxBatchV2(page) = result else {
        panic!("public V2 result");
    };
    page
}

/// Kills private fresh warning traversal, including uncharged negative-audience
/// probes that grow with retained membership history. Ordinary work stays charged.
#[test]
fn q4_private_fresh_excludes_warning_history() {
    use crate::test_support::isolation::{CostCounter, count_vm_units};
    let mut measured = Vec::new();
    for retained in [128, 8192] {
        let (store, mut db) = seeded_warning_history(retained);
        let tx = db.transaction().unwrap();
        let trace = ScopeTrace::install(&tx);
        let counter = CostCounter::default();
        let page = count_vm_units(&tx, &counter, || {
            queries::ordinary_presentation_page(&tx, &store, "i", &claim(), None, &budget())
                .unwrap()
        });
        assert_eq!(
            trace.warning_audience_probes(),
            0,
            "private fresh read skips warning audience SQL"
        );
        assert_eq!(trace.counts(), [2, 0]);
        drop(trace);
        assert_eq!((page.visited, page.lazy_scope_probes), (2, 6));
        assert_eq!(page.continuation_probes, 0);
        assert_eq!(page.retired_body, None);
        assert!(
            page.page.next_cursor.is_none(),
            "private capture exhausts ordinary sources"
        );
        assert!(matches!(page.page.items.as_slice(), [
            crate::protocol::results::InboxBatchV2Item::LazyMessage {message: first,..},
            crate::protocol::results::InboxBatchV2Item::LazyMessage {message: second,..}
        ] if first.as_str()=="msg-q4-0" && second.as_str()=="msg-q4-1"));
        measured.push(counter.units());
        eprintln!(
            "Q4 warning_history={retained} private_fresh_VM_units={}",
            counter.units()
        );
        tx.rollback().unwrap();
    }
    assert_eq!(
        measured[0], measured[1],
        "private fresh work is flat in warning audience history"
    );
}

/// Kills normalization only at fresh capture: a real public continuation still
/// carries an unfinished warning phase when private discovery resumes it.
#[test]
fn q4_private_resumed_excludes_warning_history() {
    use crate::test_support::isolation::{CostCounter, count_vm_units};
    let mut measured = Vec::new();
    for retained in [128, 8192] {
        let (store, mut db) = seeded_warning_history(retained);
        let tx = db.transaction().unwrap();
        let first = public_warning_page(&tx, &store, None, 1);
        assert!(
            matches!(first.items.as_slice(), [crate::protocol::results::InboxBatchV2Item::LazyMessage {message,..}] if message.as_str()=="msg-q4-0")
        );
        let cursor = first.next_cursor.expect("actual public continuation");
        let captured =
            crate::protocol::pagination::InboxBatchV2CursorState::decode(&cursor).unwrap();
        assert!(
            !captured.attention.next_manifest_warning,
            "public cursor retains warning discovery"
        );
        let trace = ScopeTrace::install(&tx);
        let counter = CostCounter::default();
        let page = count_vm_units(&tx, &counter, || {
            queries::ordinary_presentation_page(&tx, &store, "i", &claim(), Some(cursor), &budget())
                .unwrap()
        });
        assert_eq!(
            trace.warning_audience_probes(),
            0,
            "private resumed read skips warning audience SQL"
        );
        assert_eq!(trace.counts(), [1, 0]);
        drop(trace);
        assert_eq!((page.visited, page.lazy_scope_probes), (1, 3));
        assert!(
            page.page.next_cursor.is_none(),
            "resumed private capture exhausts ordinary sources"
        );
        assert!(
            matches!(page.page.items.as_slice(), [crate::protocol::results::InboxBatchV2Item::LazyMessage {message,..}] if message.as_str()=="msg-q4-1")
        );
        measured.push(counter.units());
        eprintln!(
            "Q4 warning_history={retained} private_resumed_VM_units={}",
            counter.units()
        );
        tx.rollback().unwrap();
    }
    assert_eq!(
        measured[0], measured[1],
        "private resumed work is flat in warning audience history"
    );
}

/// Kills applying private warning exclusion to public inbox or the independent
/// origin-warning source; the same fixture retains a real negative audience.
#[test]
fn q4_public_and_origin_warning_sources_remain_enabled() {
    let (store, mut db) = seeded_warning_history(128);
    let tx = db.transaction().unwrap();
    let trace = ScopeTrace::install(&tx);
    let page = public_warning_page(&tx, &store, None, 32);
    assert_eq!(
        trace.warning_audience_probes(),
        1,
        "public V2 checks the negative frozen audience"
    );
    drop(trace);
    let warnings = page
        .items
        .iter()
        .filter_map(|item| match item {
            crate::protocol::results::InboxBatchV2Item::Warning { warning, .. } => {
                Some(warning.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(warnings, ["q4-origin-warning"]);
    let origins = crate::store::attention::open_origin_warnings(&tx, "a-q4-0").unwrap();
    assert_eq!(
        origins
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["q4-origin-warning"]
    );
    tx.rollback().unwrap();
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
        let (store, mut db, binding) = seeded_body(128, true, 1, false);
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
