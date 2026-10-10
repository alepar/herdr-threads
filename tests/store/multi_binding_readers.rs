//! ht-54r.21: the readers that used to assume one open binding per seat (or a binding whose
//! `generation` equals the seat's) are correct for several. Included from `src/store/queries.rs`
//! as `multi_binding_readers_tests`.
//!
//! Fixture: a resolved seat `s` (mapping generation 4) holding two connected top-level agent
//! bindings (sessions A and B, enrolled through `bindings::resolve_claim`) and one disconnected
//! binding (session C), a joined thread `t` with messages. Several tests then make the bindings'
//! own `generation` stale: since V29 it is informational, so no reader may depend on it.
use super::*;
use crate::{
    ports::{PublishedSnapshot, SnapshotGenerationId},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint},
        commands::{Command, ParticipantsQuery, SeatInspectQuery, SeatsQuery, ThreadQuery},
        ids::{
            AgentId, BindingId, ExecutionId, HostBootId, HostTargetId, NativeSessionId, SeatId,
            ThreadId,
        },
        pagination::{Consistency, Page, PageRequest, StopReason},
        results::{CheckInContextDisposition, CheckInResult, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::{
        bindings::{self, BindingPlacement, FingerprintRule, ResolveRequest, resolve_claim},
        catch_up::{self, CatchUpEntry},
        connection::StoreContext,
        effective, materialization, schema, seats, summary,
    },
};
use rusqlite::{Connection, params};
use std::sync::Arc;

const EXEC: &str = "11111111-1111-4111-8111-111111111111";
const SEAT_GENERATION: u64 = 4;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1_000_000),
        cancellation: Cancellation::default(),
    }
}

fn page() -> PageRequest {
    PageRequest {
        cursor: None,
        limit: 19,
        max_bytes: 65_536,
    }
}

fn placement() -> BindingPlacement {
    BindingPlacement {
        generation: SEAT_GENERATION,
        target_generation: 1,
        target: "p".into(),
        terminal: "term".into(),
        incarnation: "inc".into(),
        host_boot: "b".into(),
        host_epoch: 1,
        execution: EXEC.into(),
        provenance: "cooperative_top_level".into(),
        observed_at: UtcMillis(5),
        registered_at: Some(UtcMillis(5)),
    }
}

fn claim(session: &str) -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: SEAT_GENERATION,
        role: CallerRole::TopLevel,
        harness: Harness::Claude,
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new(EXEC),
        target: HostTargetId::new("p"),
        agent_id: None,
        liveness: Some(LivenessFingerprint { pid: 7, start: 9 }),
    }
}

fn enroll(db: &Connection, session: &str) -> bindings::BindingRow {
    let claim = claim(session);
    let place = placement();
    resolve_claim(
        db,
        &ResolveRequest {
            claim: &claim,
            lifecycle: true,
            placement: &place,
            lineage: None,
        },
        FingerprintRule::Required,
        UtcMillis(50),
    )
    .unwrap()
    .binding()
    .clone()
}

struct Fixture {
    store: StoreContext,
    db: Connection,
}

fn fixture() -> Fixture {
    let path =
        std::env::temp_dir().join(format!("herdr-multi-binding-{}.db", uuid::Uuid::new_v4()));
    let store = StoreContext::new(path, Arc::new(FixedClock));
    let db = store.open_writer().unwrap();
    db.execute_batch(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,30);
         INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',4,1,0);
         INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);
         INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t','s','joined',1);
         INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m1','t',1,'ordinary','b',1,1),('i','m2','t',2,'ordinary','b',2,2);
         UPDATE threads SET next_sequence=3 WHERE id='t';",
    )
    .unwrap();
    let a = enroll(&db, "sess-A");
    let b = enroll(&db, "sess-B");
    let c = enroll(&db, "sess-C");
    assert_ne!(a.binding_id, b.binding_id);
    assert!(
        bindings::disconnect(
            &db,
            &c.binding_id,
            bindings::DisconnectReason::ProcessExit,
            UtcMillis(60)
        )
        .unwrap()
    );
    Fixture { store, db }
}

/// Makes every binding's own `generation` disagree with the seat's: the V29 contract says it is
/// informational, so no reader may use it to find "the current binding".
fn stale_binding_generations(db: &Connection) {
    db.execute("UPDATE occupant_bindings SET generation=3", [])
        .unwrap();
}

fn open_count(db: &Connection) -> i64 {
    db.query_row(
        "SELECT count(*) FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn fixture_holds_three_open_bindings_one_disconnected() {
    let f = fixture();
    assert_eq!(open_count(&f.db), 3);
    let connected = bindings::list_connected(&f.db, &SeatId::new("s")).unwrap();
    assert_eq!(connected.len(), 2, "A and B are connected, C is not");
}

#[test]
fn participants_and_seat_views_list_the_seat_once() {
    let f = fixture();
    stale_binding_generations(&f.db);
    let q = |command: Command| query(&f.store, "i", &command, &budget()).unwrap();
    let CommandResult::Participants(participants) = q(Command::Participants(ParticipantsQuery {
        thread: ThreadId::new("t"),
        page: page(),
        caller: None,
    })) else {
        panic!("wrong result")
    };
    assert_eq!(
        participants
            .items
            .iter()
            .map(|p| p.seat.as_str())
            .collect::<Vec<_>>(),
        vec!["s"],
        "three open bindings yield one participant row"
    );
    let CommandResult::Thread(details) = q(Command::Thread(ThreadQuery {
        thread: ThreadId::new("t"),
        page: page(),
        caller: Some(SeatId::new("s")),
    })) else {
        panic!("wrong result")
    };
    assert_eq!(
        details
            .participants
            .items
            .iter()
            .filter(|p| p.is_self)
            .count(),
        1,
        "thread show marks the caller's seat once"
    );
    let CommandResult::Seats(seats) = q(Command::Seats(SeatsQuery {
        page: page(),
        target: None,
        include_retired: false,
    })) else {
        panic!("wrong result")
    };
    assert_eq!(
        seats
            .items
            .iter()
            .filter(|s| s.seat.as_str() == "s")
            .count(),
        1,
        "the seat directory lists the seat once"
    );
    let CommandResult::SeatInspect(inspection) = q(Command::SeatInspect(SeatInspectQuery {
        seat: SeatId::new("s"),
        page: PageRequest { limit: 1, ..page() },
    })) else {
        panic!("wrong result")
    };
    let open = inspection.open_binding.expect("the seat is bound");
    assert_eq!(open.harness, "claude");
    assert_eq!(open.target, HostTargetId::new("p"));
}

fn offer(claim: &CallerClaim) -> CommandResult {
    fn empty<T>() -> Page<T> {
        Page {
            items: vec![],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        }
    }
    CommandResult::CheckedIn(CheckInResult {
        context_disposition: CheckInContextDisposition::Current,
        context: claim.clone(),
        seat: claim.seat.clone(),
        offered_through: None,
        warning_count: 0,
        warning_count_has_more: false,
        warnings: empty(),
        notices: Default::default(),
        inbox: empty(),
    })
}

fn disposition(f: &mut Fixture, claim: &CallerClaim) -> CheckInContextDisposition {
    let tx = f.db.transaction().unwrap();
    let CommandResult::CheckedIn(result) =
        seats::present_check_in_result(&tx, offer(claim)).unwrap()
    else {
        panic!("wrong result")
    };
    result.context_disposition
}

#[test]
fn check_in_result_is_current_for_each_binding() {
    let mut f = fixture();
    for session in ["sess-A", "sess-B", "sess-C"] {
        assert_eq!(
            disposition(&mut f, &claim(session)),
            CheckInContextDisposition::Current,
            "{session} holds an open binding at the seat generation"
        );
    }
    // The binding's own generation and the pane's execution are not compared.
    stale_binding_generations(&f.db);
    let mut other_execution = claim("sess-B");
    other_execution.execution = ExecutionId::new("22222222-2222-4222-8222-222222222222");
    assert_eq!(
        disposition(&mut f, &other_execution),
        CheckInContextDisposition::Current
    );
    // A key with no open binding is historical.
    assert_eq!(
        disposition(&mut f, &claim("sess-unknown")),
        CheckInContextDisposition::Historical
    );
    let mut subagent = claim("sess-A");
    subagent.role = CallerRole::Subagent;
    subagent.agent_id = Some(AgentId::new("a1"));
    assert_eq!(
        disposition(&mut f, &subagent),
        CheckInContextDisposition::Historical,
        "the subagent key differs from the top-level key"
    );
    // The seat fence: a claim older than the seat's mapping generation is historical.
    let mut behind = claim("sess-A");
    behind.binding_generation = SEAT_GENERATION - 1;
    assert_eq!(
        disposition(&mut f, &behind),
        CheckInContextDisposition::Historical
    );
    // Ending A's binding makes exactly A historical; B stays current.
    f.db.execute(
        "UPDATE occupant_bindings SET ended_at=70 WHERE native_session='sess-A'",
        [],
    )
    .unwrap();
    assert_eq!(
        disposition(&mut f, &claim("sess-A")),
        CheckInContextDisposition::Historical
    );
    assert_eq!(
        disposition(&mut f, &claim("sess-B")),
        CheckInContextDisposition::Current
    );
}

#[test]
fn open_binding_role_reports_agent_for_a_multi_binding_seat() {
    let f = fixture();
    let seat = SeatId::new("s");
    assert_eq!(
        schema::open_binding_role(&f.db, &seat).unwrap(),
        Some("agent"),
        "several open agent bindings read as agent, not as a store error"
    );
    f.db.execute("UPDATE occupant_bindings SET ended_at=70", [])
        .unwrap();
    assert_eq!(schema::open_binding_role(&f.db, &seat).unwrap(), None);
    f.db.execute_batch(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('h','i','resolved','native','ph',1,1,0);
         INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('h',1,1,'ph','b',1,'human','','e','cooperative_top_level',0,0,'term','inc');",
    )
    .unwrap();
    assert_eq!(
        schema::open_binding_role(&f.db, &SeatId::new("h")).unwrap(),
        Some("human")
    );
}

fn epochs(db: &Connection) -> Vec<(String, i64, i64, bool)> {
    let mut stmt = db
        .prepare("SELECT native_session,host_epoch,target_generation,ended_at IS NULL FROM occupant_bindings WHERE seat_id='s' ORDER BY ordinal")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn publication(epoch: u64) -> PublishedSnapshot {
    PublishedSnapshot {
        id: SnapshotGenerationId::store_issued("g1".into()),
        instance: "i".into(),
        boot: HostBootId::new("b"),
        epoch,
        observation_sequence: 1,
        incarnation: "inc".into(),
        target_count: 1,
        invalidation_revision: 0,
    }
}

#[test]
fn carry_binding_forward_carries_every_registered_binding() {
    let mut f = fixture();
    // An ended binding of the seat must not move.
    let ended = enroll(&f.db, "sess-ended");
    f.db.execute(
        "UPDATE occupant_bindings SET ended_at=70 WHERE binding_id=?1",
        [ended.binding_id.as_str()],
    )
    .unwrap();
    stale_binding_generations(&f.db);
    let tx = f.db.transaction().unwrap();
    let carried = seats::carry_binding_forward(
        &tx,
        "i",
        UtcMillis(100),
        &SeatId::new("s"),
        SEAT_GENERATION,
        &publication(2),
        "p",
        2,
    )
    .unwrap();
    assert!(carried);
    tx.commit().unwrap();
    assert_eq!(
        epochs(&f.db),
        vec![
            ("sess-A".to_owned(), 2, 2, true),
            ("sess-B".to_owned(), 2, 2, true),
            ("sess-C".to_owned(), 2, 2, true),
            ("sess-ended".to_owned(), 1, 1, false),
        ],
        "A, B and the disconnected C move to the new epoch; the ended row stays"
    );
    let anchors: Vec<(i64, String)> = {
        let mut stmt = f
            .db
            .prepare("SELECT binding_generation,observation_provenance FROM seat_availability WHERE seat_id='s'")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        anchors,
        vec![(SEAT_GENERATION as i64, "cooperative_top_level".to_owned())],
        "one availability anchor per seat, at the seat generation"
    );
    // Nothing left to move.
    let tx = f.db.transaction().unwrap();
    assert!(
        !seats::carry_binding_forward(
            &tx,
            "i",
            UtcMillis(101),
            &SeatId::new("s"),
            SEAT_GENERATION,
            &publication(2),
            "p",
            2,
        )
        .unwrap()
    );
}

fn insert_offer(db: &Connection, generation: i64, execution: &str, through: i64) {
    db.execute(
        "INSERT INTO warning_offer(seat_id,binding_generation,execution_id,offered_through_seq) VALUES ('s',?1,?2,?3) ON CONFLICT(seat_id) DO UPDATE SET binding_generation=excluded.binding_generation,execution_id=excluded.execution_id,offered_through_seq=excluded.offered_through_seq",
        params![generation, execution, through],
    )
    .unwrap();
}

#[test]
fn materialization_and_summary_readers_see_the_seat() {
    let f = fixture();
    stale_binding_generations(&f.db);

    // materialization::offered: the seat's offer counts while some open binding holds its
    // execution and the seat has not been rebound.
    insert_offer(&f.db, SEAT_GENERATION as i64, EXEC, 7);
    assert!(materialization::offered(&f.db, "s", 7).unwrap());
    assert!(
        !materialization::offered(&f.db, "s", 8).unwrap(),
        "above the offer"
    );
    insert_offer(&f.db, SEAT_GENERATION as i64 - 1, EXEC, 7);
    assert!(
        !materialization::offered(&f.db, "s", 7).unwrap(),
        "an offer from before a rebind does not count"
    );
    insert_offer(&f.db, SEAT_GENERATION as i64, "gone-execution", 7);
    assert!(
        !materialization::offered(&f.db, "s", 7).unwrap(),
        "no open binding holds the offer's execution"
    );

    // The inbox v2 snapshot fence is the seat generation and one open binding's execution; a
    // snapshot stays valid while another session enrolls and fails once nothing holds it.
    let (generation, execution) = inbox_v2::binding(&f.db, &SeatId::new("s"))
        .unwrap()
        .expect("bound seat");
    assert_eq!((generation, execution.as_str()), (SEAT_GENERATION, EXEC));
    let seat = SeatId::new("s");
    assert!(inbox_v2::fence_holds(&f.db, &seat, Some(generation), Some(&execution)).unwrap());
    assert!(!inbox_v2::fence_holds(&f.db, &seat, Some(generation - 1), Some(&execution)).unwrap());
    assert!(
        !inbox_v2::fence_holds(&f.db, &seat, Some(generation), Some(&ExecutionId::new("x")))
            .unwrap()
    );
    assert!(!inbox_v2::fence_holds(&f.db, &seat, None, None).unwrap());
    // Ending only A leaves B and C holding the execution.
    f.db.execute(
        "UPDATE occupant_bindings SET ended_at=70 WHERE native_session='sess-A'",
        [],
    )
    .unwrap();
    assert!(inbox_v2::fence_holds(&f.db, &seat, Some(generation), Some(&execution)).unwrap());
    f.db.execute("UPDATE occupant_bindings SET ended_at=71", [])
        .unwrap();
    assert!(!inbox_v2::fence_holds(&f.db, &seat, Some(generation), Some(&execution)).unwrap());
    assert!(inbox_v2::fence_holds(&f.db, &seat, None, None).unwrap());
    assert_eq!(inbox_v2::binding(&f.db, &seat).unwrap(), None);
}

#[test]
fn summary_caller_is_current_per_session_binding() {
    let f = fixture();
    stale_binding_generations(&f.db);
    let thread = ThreadId::new("t");
    let current = |claim: &CallerClaim| {
        summary::check_caller(&f.db, "i", claim, &thread)
            .unwrap()
            .current
    };
    assert!(current(&claim("sess-A")));
    assert!(current(&claim("sess-B")));
    assert_eq!(
        summary::check_caller(&f.db, "i", &claim("sess-B"), &thread)
            .unwrap()
            .binding,
        Some(binding_of(&f.db, "sess-B")),
        "the catch-up row belongs to the claim's own session binding"
    );
    assert!(!current(&claim("sess-unknown")), "no binding for the key");
    let mut behind = claim("sess-A");
    behind.binding_generation = SEAT_GENERATION - 1;
    assert!(!current(&behind), "the seat was rebound since the claim");
    // A summary worker acts for its session's top-level binding.
    let mut worker = claim("sess-B");
    worker.role = CallerRole::Subagent;
    worker.agent_id = Some(AgentId::new("w1"));
    assert!(current(&worker));
    f.db.execute(
        "UPDATE occupant_bindings SET ended_at=70 WHERE native_session='sess-B'",
        [],
    )
    .unwrap();
    assert!(!current(&claim("sess-B")));
    assert!(current(&claim("sess-A")));
    let error = summary::check_caller(&f.db, "other-instance", &claim("sess-A"), &thread)
        .err()
        .expect("instance mismatch is refused");
    assert_eq!(error.code, ErrorCode::CallerUnverified);
}

fn binding_of(db: &Connection, session: &str) -> BindingId {
    BindingId::parse(
        db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE native_session=?1",
            [session],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap()
}

/// ht-54r.22: catch-up rows belong to a binding. Entry needs an open binding of the seat (every
/// seat generation bump ends the seat's open bindings), and only an ended binding's rows are
/// superseded: another session's check-in leaves them alone.
#[test]
fn catch_up_entry_and_supersede_are_per_binding() {
    let mut f = fixture();
    stale_binding_generations(&f.db);
    let seat = SeatId::new("s");
    let thread = ThreadId::new("t");
    let (a, b) = (binding_of(&f.db, "sess-A"), binding_of(&f.db, "sess-B"));
    let settings = crate::protocol::summary::SummarySettings::default();
    let tx = f.db.transaction().unwrap();
    let frontier = catch_up::enter_or_keep(
        &tx,
        &CatchUpEntry {
            binding: &a,
            seat: &seat,
            thread: &thread,
            frontier_seq: 2,
            now: UtcMillis(100),
        },
        &settings,
    )
    .unwrap();
    assert_eq!(frontier, 2);
    // A binding of another seat is refused.
    let other_seat = SeatId::new("other");
    let error = catch_up::enter_or_keep(
        &tx,
        &CatchUpEntry {
            binding: &b,
            seat: &other_seat,
            thread: &thread,
            frontier_seq: 2,
            now: UtcMillis(100),
        },
        &settings,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::CallerUnverified);
    tx.commit().unwrap();
    let state = |db: &Connection| -> String {
        db.query_row(
            "SELECT state FROM catch_up WHERE binding_id=?1 AND thread_id='t'",
            [a.as_str()],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(state(&f.db), "active");
    // Another session's lifecycle check-in supersedes nothing: A is still open.
    let tx = f.db.transaction().unwrap();
    assert_eq!(
        catch_up::supersede_ended_bindings(&tx, &seat, UtcMillis(110)).unwrap(),
        0
    );
    assert_eq!(
        catch_up::supersede_stale(&tx, &a, UtcMillis(110)).unwrap(),
        0
    );
    tx.commit().unwrap();
    assert_eq!(state(&f.db), "active");
    // Once A ends its row is superseded; B's entry is unaffected.
    f.db.execute(
        "UPDATE occupant_bindings SET ended_at=115 WHERE binding_id=?1",
        [a.as_str()],
    )
    .unwrap();
    let tx = f.db.transaction().unwrap();
    let error = catch_up::enter_or_keep(
        &tx,
        &CatchUpEntry {
            binding: &a,
            seat: &seat,
            thread: &thread,
            frontier_seq: 2,
            now: UtcMillis(116),
        },
        &settings,
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::CallerUnverified,
        "an ended binding cannot enter"
    );
    assert_eq!(
        catch_up::supersede_ended_bindings(&tx, &seat, UtcMillis(120)).unwrap(),
        1
    );
    tx.commit().unwrap();
    assert_eq!(state(&f.db), "ended");
}

#[test]
fn effective_non_human_reader_handles_multiple_bindings() {
    let f = fixture();
    // Several open agent bindings are not a human occupant: no waiver without a cutoff.
    assert!(!effective::human_receipt_waived(&f.db, "s", Some(1)).unwrap());
    f.db.execute(
        "INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('s',5,1,0)",
        [],
    )
    .unwrap();
    assert!(effective::human_receipt_waived(&f.db, "s", Some(5)).unwrap());
    assert!(!effective::human_receipt_waived(&f.db, "s", Some(6)).unwrap());
    // A human binding alone waives (the human branch itself is ht-54r.4's).
    f.db.execute_batch(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('h','i','resolved','native','ph',1,1,0);
         INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('h',1,1,'ph','b',1,'human','','e','cooperative_top_level',0,0,'term','inc');",
    )
    .unwrap();
    assert!(effective::human_receipt_waived(&f.db, "h", None).unwrap());
}
