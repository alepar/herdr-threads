//! Two-store convergence of `seat.created` (fact-kinds design section 4):
//! the remote row, discovery without prior interaction, and own-seat restore
//! after a wipe.

use herdr_threads::ports::{ClosureEvidence, RetirementJob, WorkAdmission};
use herdr_threads::protocol::ids::{HostBootId, HostTargetId, RetirementJobId, SeatId, ThreadId};
use herdr_threads::protocol::results::{ApiError, ErrorCode};
use herdr_threads::store::control;
use herdr_threads::sync::facts::seat::SeatRetiredPayload;
use herdr_threads::sync::hlc::HlcStamp;
use herdr_threads::sync::journal_port::JournalPort;
use herdr_threads::test_support::{fact_exchange, write_guard};

use crate::facts_seat::{Node, budget, hold_off, hold_on, registry, retirement_rows};

fn seat_created_ms(node: &Node) -> i64 {
    let hlc: i64 = node.query(
        "SELECT hlc FROM journal WHERE kind='seat.created' ORDER BY ingest_seq",
        |r| r.get(0),
    )[0];
    i64::try_from(HlcStamp::from_u64(u64::try_from(hlc).unwrap()).ms()).unwrap()
}

#[test]
fn b_holds_a_seat_as_remote_after_sync() {
    let reg = registry();
    let (a, b) = (Node::with(41, reg), Node::with(42, reg));
    let seat = a.mint_seat(0);
    a.add_local_seat(&seat);
    a.enroll(&seat, "codex", "session-1").unwrap();
    fact_exchange::sync(&a.store, &b.store);
    let row = b.seat(&seat).expect("B holds A's seat");
    assert_eq!(row.0, "remote");
    assert!(!row.1, "is_local = 0");
    assert_eq!(row.2.as_deref(), Some("agent"));
    assert!(row.3, "published = 1");
    assert_eq!(
        row.4,
        seat_created_ms(&a),
        "created_at is the origin's sent-at"
    );
    assert_eq!(row.5, a.id.to_string(), "homed on A");
    // A's own row is the local one, now published.
    let own = a.seat(&seat).unwrap();
    assert_eq!((own.0.as_str(), own.1, own.3), ("resolved", true, true));
}

#[test]
fn seat_enrolled_on_b_listed_on_a_without_interaction() {
    let reg = registry();
    let (a, b) = (Node::with(43, reg), Node::with(44, reg));
    let seat = b.mint_seat(0);
    b.add_local_seat(&seat);
    // Resolved to an occupant on B; nothing ever references it in a fact.
    b.enroll(&seat, "human", "session-h").unwrap();
    assert_eq!(
        b.journal_kinds()
            .iter()
            .filter(|k| *k != "node.genesis")
            .count(),
        1
    );
    fact_exchange::sync(&a.store, &b.store);
    let row = a.seat(&seat).expect("A lists B's seat");
    assert_eq!((row.0.as_str(), row.1, row.3), ("remote", false, true));
    assert_eq!(
        row.2.as_deref(),
        Some("human"),
        "kind from the binding role"
    );
}

/// A wiped store of the same node: same identity, another incarnation.
fn wiped(seed: u8, host: bool) -> Node {
    Node::open(seed, 999, registry(), host)
}

#[test]
fn own_seat_restore_unresolved_on_latest_host() {
    let reg = registry();
    let old = Node::with(45, reg);
    let seat = old.mint_seat(0);
    old.add_local_seat(&seat);
    old.enroll(&seat, "codex", "session-1").unwrap();

    let fresh = wiped(45, false);
    assert_eq!(fresh.id, old.id, "the same node");
    let ctx = fresh.ctx();
    ctx.attach_host("h-low-id").unwrap();
    ctx.attach_host("h-high-id").unwrap();
    // The lower host_id is the latest attached, so ordering by id cannot pass.
    fresh
        .raw()
        .execute_batch(
            "UPDATE herdr_hosts SET last_attached_at=300 WHERE endpoint='h-low-id'; \
             UPDATE herdr_hosts SET last_attached_at=200 WHERE endpoint='h-high-id';",
        )
        .unwrap();
    let latest: i64 = fresh.query(
        "SELECT host_id FROM herdr_hosts WHERE endpoint='h-low-id'",
        |r| r.get(0),
    )[0];

    fact_exchange::sync(&old.store, &fresh.store);

    let rows = fresh.query(
        &format!(
            "SELECT state,unresolved_reason,is_local,published,host_id,remote_kind FROM seats WHERE id='{}'",
            seat.as_str()
        ),
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        },
    );
    assert_eq!(
        rows,
        vec![(
            "unresolved".to_owned(),
            Some("other".to_owned()),
            true,
            true,
            latest,
            None
        )]
    );
    assert!(
        fresh
            .journal()
            .iter()
            .filter(|(k, _, _)| k == "seat.created")
            .all(|(_, state, _)| state == "applied")
    );
}

/// A wiped `fresh` store without a host that pulled its old self's log: the
/// own `seat.created` waits for a host and the `thread.created` behind it
/// parks on the seat. Returns `(old, fresh, seat, thread)`.
fn wiped_without_host(seed: u8) -> (Node, Node, SeatId, ThreadId) {
    let reg = registry();
    let old = Node::with(seed, reg);
    let seat = old.mint_seat(0);
    old.add_local_seat(&seat);
    let thread = old.create_thread(&seat, "t", None).unwrap();

    let fresh = wiped(seed, false);
    assert!(
        fresh
            .query("SELECT 1 FROM herdr_hosts", |r| r.get::<_, i64>(0))
            .is_empty(),
        "no host row"
    );
    fact_exchange::sync(&old.store, &fresh.store);

    let rows = fresh.query(
        "SELECT kind,apply_state,parked_on,apply_error FROM journal \
         WHERE kind IN ('seat.created','thread.created')",
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        },
    );
    let seat_row = rows.iter().find(|r| r.0 == "seat.created").unwrap();
    assert_eq!(seat_row.1, "parked_dep");
    assert_eq!(seat_row.2.as_deref(), Some("wait:host"));
    assert!(
        seat_row
            .3
            .as_deref()
            .unwrap()
            .contains("own seat restore needs an attached host"),
        "{seat_row:?}"
    );
    let thread_row = rows.iter().find(|r| r.0 == "thread.created").unwrap();
    assert_eq!(thread_row.1, "parked_dep");
    assert_eq!(thread_row.2.as_deref(), Some(seat.as_str()));
    assert!(fresh.seat(&seat).is_none(), "no row inserted");
    (old, fresh, seat, thread)
}

/// The restored seat is an unresolved local published seat on the host, the
/// dependent applied, and nothing is left parked or failed.
fn assert_restored(fresh: &Node, seat: &SeatId, thread: &ThreadId) {
    let host: i64 = fresh.query("SELECT host_id FROM herdr_hosts", |r| r.get(0))[0];
    let rows = fresh.query(
        &format!(
            "SELECT state,is_local,published,host_id FROM seats WHERE id='{}'",
            seat.as_str()
        ),
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    );
    assert_eq!(rows, vec![("unresolved".to_owned(), true, true, host)]);
    assert!(fresh.thread_row(thread).is_some(), "the dependent applied");
    assert!(
        fresh
            .journal()
            .iter()
            .all(|(_, state, _)| state != "parked_dep" && state != "failed"),
        "{:?}",
        fresh.journal()
    );
}

#[test]
fn own_seat_restore_without_host_waits_and_applies_on_attach() {
    let (_old, fresh, seat, thread) = wiped_without_host(46);
    // Still no host: a further pass changes nothing and does not loop.
    let progress = fresh.store.apply_pending(&budget()).unwrap();
    assert!(!progress.has_more, "{progress:?}");
    assert!(fresh.seat(&seat).is_none());

    fresh.ctx().attach_host("h-1").unwrap();
    loop {
        if !fresh.store.apply_pending(&budget()).unwrap().has_more {
            break;
        }
    }
    assert_restored(&fresh, &seat, &thread);
}

#[test]
fn own_seat_restore_waits_until_startup_replay_with_a_host() {
    let (_old, fresh, seat, thread) = wiped_without_host(47);
    let progress = fresh.store.replay_startup(&budget()).unwrap();
    assert_eq!(progress.applied, 0, "no host: startup leaves it waiting");
    assert!(fresh.seat(&seat).is_none());

    fresh.ctx().attach_host("h-1").unwrap();
    loop {
        if !fresh.store.replay_startup(&budget()).unwrap().has_more {
            break;
        }
    }
    assert_restored(&fresh, &seat, &thread);
}

// ---- seat.retired ------------------------------------------------------------

/// Makes the closure proof of `seat` current on `node`'s host (what an
/// absent-target enumeration leaves) and returns it.
pub fn closure_proof(node: &Node, seat: &SeatId) -> ClosureEvidence {
    let target = format!("p-{}", seat.as_str());
    node.raw()
        .execute_batch(&format!(
            "UPDATE herdr_hosts SET host_boot='boot-1',host_epoch=1; \
             INSERT OR IGNORE INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance) \
               SELECT host_id,'{target}','boot-1',1,1,1,0,'fresh' FROM herdr_hosts;"
        ))
        .unwrap();
    ClosureEvidence {
        host_boot: HostBootId::new("boot-1"),
        epoch: 1,
        target: HostTargetId::new(target),
        generation: 1,
    }
}

/// Starts the retirement of `seat` on its home node, with the write guard
/// armed on the writer.
pub fn begin_retirement(node: &Node, seat: &SeatId) -> Result<RetirementJob, ApiError> {
    let proof = closure_proof(node, seat);
    let mut conn = node.raw();
    let _guard = write_guard::arm(&conn);
    control::begin_retirement(node.ctx(), &mut conn, seat.clone(), proof)
}

fn retired_payloads(node: &Node) -> Vec<SeatRetiredPayload> {
    node.query(
        "SELECT payload FROM journal WHERE kind='seat.retired' ORDER BY ingest_seq",
        |r| r.get::<_, Vec<u8>>(0),
    )
    .into_iter()
    .map(|b| serde_json::from_slice(&b).expect("a seat.retired payload"))
    .collect()
}

/// A remote seat's obligations on `a`: a joined thread with a pending ACK
/// receipt and a thread it is invited to with a pending invitation.
fn add_obligations(a: &Node, seat: &SeatId) {
    let s = seat.as_str();
    a.raw()
        .execute_batch(&format!(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled,next_sequence) \
               VALUES('tRetire0001',(SELECT id FROM local_node),'t','',1,1,1,2), \
                     ('tRetire0002',(SELECT id FROM local_node),'t','',1,1,1,1); \
             INSERT INTO memberships(thread_id,seat_id,episode,state,joined_at,voluntary_state) \
               VALUES('tRetire0001','{s}',1,'joined',1,'joined'); \
             INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) \
               VALUES('tRetire0001','{s}',1,1); \
             INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) \
               VALUES('tRetire0002','{s}',1,'invited','invited'); \
             INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node,inviter_kind) \
               VALUES('iRetire00001','tRetire0002','{s}',1,'pending',1,1,1000,2000,(SELECT id FROM local_node),'operator'); \
             INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq,origin_node) \
               VALUES((SELECT id FROM local_node),'mRetire00001','tRetire0001',1,'ordinary','b',0,1,(SELECT id FROM local_node)); \
             INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) \
               VALUES('mRetire00001','tRetire0001','{s}','pending',90,0,90);"
        ))
        .unwrap();
}

#[test]
fn published_seat_retired_on_b_retires_on_a_and_settles() {
    let reg = registry();
    let (a, b) = (Node::with(47, reg), Node::with(48, reg));
    let seat = b.mint_seat(0);
    b.add_local_seat(&seat);
    b.enroll(&seat, "codex", "session-1").unwrap();
    fact_exchange::sync(&a.store, &b.store);
    add_obligations(&a, &seat);

    let job = begin_retirement(&b, &seat).unwrap();
    assert_eq!(b.seat(&seat).unwrap().0, "retired", "B's fence retired it");
    assert_eq!(b.count_kind("seat.retired"), 1);
    assert_eq!(retired_payloads(&b).len(), 1);
    let authored = &retired_payloads(&b)[0];
    assert_eq!(authored.seat, seat);
    assert_eq!(authored.retired_at, job.retired_at.0, "the cutover");
    fact_exchange::sync(&a.store, &b.store);

    // The replicated part: the row and a `replicated` job row without closure columns.
    assert_eq!(a.seat(&seat).unwrap().0, "retired");
    assert_eq!(
        retirement_rows(&a),
        vec![(
            seat.as_str().to_owned(),
            "replicated".to_owned(),
            job.retired_at.0,
            None
        )]
    );
    assert_eq!(
        retirement_rows(&b)[0].1,
        "local",
        "B keeps its closure-backed row"
    );
    // Nothing settled yet: that is the job's work, in the deadline lane.
    let pending = |sql: &str| a.query(sql, |r| r.get::<_, String>(0));
    assert_eq!(
        pending("SELECT state FROM invitations"),
        vec!["pending".to_owned()]
    );

    let job_id = RetirementJobId::new(
        pending("SELECT id FROM retirements WHERE origin='replicated'").remove(0),
    );
    let mut conn = a.raw();
    let mut turns = 0;
    loop {
        let progress = control::advance_retirement(
            a.ctx(),
            &mut conn,
            job_id.clone(),
            WorkAdmission::Background,
            &budget(),
        )
        .unwrap();
        turns += 1;
        assert!(turns < 50, "the job must finish");
        if progress.complete {
            break;
        }
    }
    drop(conn);
    assert_eq!(
        pending("SELECT state FROM invitations"),
        vec!["recipient_retired".to_owned()]
    );
    assert_eq!(
        pending("SELECT state FROM receipts"),
        vec!["recipient_retired".to_owned()]
    );
    assert_eq!(
        pending("SELECT state FROM memberships ORDER BY thread_id"),
        vec!["retired".to_owned(), "retired".to_owned()],
        "memberships closed"
    );
    assert_eq!(
        a.query(
            "SELECT count(*) FROM membership_intervals WHERE left_seq IS NULL",
            |r| r.get::<_, i64>(0)
        ),
        vec![0],
        "the open interval closed"
    );
    assert_eq!(
        pending("SELECT status FROM retirements"),
        vec!["complete".to_owned()]
    );
    assert_eq!(
        a.query("SELECT count(*) FROM retirement_audits", |r| r
            .get::<_, i64>(0)),
        vec![2],
        "one audit event per membership thread"
    );
}

#[test]
fn unpublished_seat_retirement_authors_nothing() {
    let reg = registry();
    let (a, b) = (Node::with(49, reg), Node::with(50, reg));
    let seat = b.mint_seat(0);
    b.add_local_seat(&seat);
    // Never enrolled, so never published.
    begin_retirement(&b, &seat).unwrap();
    assert_eq!(b.seat(&seat).unwrap().0, "retired");
    assert_eq!(retirement_rows(&b).len(), 1);
    assert_eq!(b.count_kind("seat.retired"), 0);
    assert_eq!(
        b.count_kind("seat.created"),
        0,
        "retirement does not publish"
    );
    fact_exchange::sync(&a.store, &b.store);
    assert!(a.seat(&seat).is_none());
}

#[test]
fn retirement_waits_for_own_history_hold_then_authors_once() {
    let reg = registry();
    let b = Node::with(63, reg);
    let seat = b.mint_seat(0);
    b.add_local_seat(&seat);
    b.enroll(&seat, "codex", "session-1").unwrap();
    assert_eq!(b.count_kind("seat.created"), 1);

    hold_on(&b);
    let error = begin_retirement(&b, &seat).unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceBusy, "retryable");
    assert_eq!(error.detail, "own history catching up");
    assert_eq!(b.count_kind("seat.retired"), 0, "no seat.retired authored");
    assert_eq!(
        b.seat(&seat).unwrap().0,
        "resolved",
        "the fence did not half apply"
    );
    assert!(retirement_rows(&b).is_empty(), "no job row to retry from");

    hold_off(&b);
    begin_retirement(&b, &seat).unwrap();
    assert_eq!(
        b.count_kind("seat.retired"),
        1,
        "exactly one after the hold"
    );
    assert_eq!(b.seat(&seat).unwrap().0, "retired");
    // The job already exists: a repeated start authors nothing more.
    begin_retirement(&b, &seat).unwrap();
    assert_eq!(b.count_kind("seat.retired"), 1);
}
