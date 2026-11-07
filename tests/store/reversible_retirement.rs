//! Reversible retirement against the real store (ht-ma6i.3, spec 4.3): the disposition, un-retire,
//! finalization and each write-driving path that composes the frozen fragment. Mounted from
//! `src/store/seats/reversible.rs`. The plan-driven closes and the agent-exit rule are in
//! `tests/store/restored_pane.rs`, `--replace` in `tests/store/control.rs`.
use super::*;
use crate::{
    ports::{
        ClosureEvidence, RetirementStart, ServiceAuthorityGate, ServiceConnectionAuthority,
        ServiceDecisionGuard, ServiceWriteTransactionProof, StorePort, WorkAdmission,
    },
    protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::OperatorRetire,
        ids::{ContinuityKey, HostBootId, HostTargetId, MessageId, OperationId, ThreadId},
        results::ContinuityConflictKind,
        service::{
            EnsureManagedThread, FinalizeRetirement, InvitationConstraint, ReserveContinuityKey,
            ServiceInvite, ServiceOperation, ServiceSend, VoluntaryMembershipState,
        },
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    store::{
        SqliteStore, StoreSettings, archival,
        effective::{self, EffectiveReceiptState},
        invitation_due, materialization, materialize, poke, queries, receipts, seats,
        service_substrate,
    },
    sync::facts::seat::SeatRetiredPayload,
    test_support::history,
};
use rusqlite::Connection;
use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicI64, Ordering},
    },
    time::Instant,
};

#[path = "reversible_reads.rs"]
mod reversible_reads;

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";

/// The monotonic clock stands still until a test starts it through the second field.
struct TestClock(AtomicI64, OnceLock<Instant>);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(
            1 + self
                .1
                .get()
                .map_or(0, |start| start.elapsed().as_millis() as u64),
        )
    }
}

struct Gate(ServiceAuthorId);
struct Guard(ServiceAuthorId);
impl ServiceDecisionGuard for Guard {
    fn author(&self) -> &ServiceAuthorId {
        &self.0
    }
}
impl ServiceAuthorityGate for Gate {
    fn register(
        &self,
        instance: &str,
        boot: &str,
        author: ServiceAuthorId,
    ) -> Result<ServiceConnectionAuthority, ApiError> {
        Ok(ServiceConnectionAuthority::new(
            instance.into(),
            boot.into(),
            1,
            author,
        ))
    }
    fn decision_guard<'a>(
        &'a self,
        _: &ServiceWriteTransactionProof,
        _: &ServiceConnectionAuthority,
    ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
        Ok(Box::new(Guard(self.0.clone())))
    }
    fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
        true
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

struct Fx {
    store: SqliteStore,
    connection: ServiceConnectionAuthority,
    gate: Gate,
    clock: Arc<TestClock>,
    path: PathBuf,
}
impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A resolved seat with a registered binding and a confirmed observation, joined to thread `t`.
fn seed_seat(db: &Connection, seat: &str) {
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence,created_at,home_node,is_local,host_id) VALUES (?1,?2,'resolved','native',?1,1,1,'term-'||?1,'inc','coherent_enumeration','b',1,1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", params![seat, INSTANCE]).unwrap();
    db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)", [seat]).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation,registered_at) VALUES (?1,1,1,?1,'b',1,'codex','n',?2,'cooperative_top_level',0,'term-'||?1,'inc',0)", params![seat, history::fixture_execution(seat)]).unwrap();
    db.execute("INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES ('t',?1,'joined','joined',1)", [seat]).unwrap();
    db.execute(
        "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)",
        [seat],
    )
    .unwrap();
    db.execute(
        "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'",
        [],
    )
    .unwrap();
}

/// Thread `t` with `seats` joined; the clock reads 100.
fn fx(seats: &[&str]) -> Fx {
    let path = crate::test_support::scratch::temp_path(format!(
        "reversible-retirement-{}.db",
        uuid::Uuid::new_v4()
    ));
    let clock = Arc::new(TestClock(AtomicI64::new(100), OnceLock::new()));
    let context = StoreContext::new(path.clone(), clock.clone()).with_test_host();
    let db = context.open_writer().unwrap();
    db.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let author = service_substrate::ensure_reserved_author(&tx, INSTANCE, UtcMillis(0)).unwrap();
    tx.commit().unwrap();
    drop(db);
    let store = SqliteStore::new(context, INSTANCE, StoreSettings::default()).unwrap();
    let fixture = Fx {
        store,
        connection: ServiceConnectionAuthority::new(
            INSTANCE.into(),
            "boot".into(),
            1,
            author.clone(),
        ),
        gate: Gate(author),
        clock,
        path,
    };
    fixture
        .op(ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: ThreadId::new("t"),
            topic: "topic".into(),
            goal: "goal".into(),
            operation: OperationId::new("ensure-t"),
        }))
        .unwrap();
    let db = fixture.db();
    for seat in seats {
        seed_seat(&db, seat);
    }
    fixture
}

impl Fx {
    fn db(&self) -> Connection {
        self.store.context.open_writer().unwrap()
    }
    fn at(&self, now: i64) {
        self.clock.0.store(now, Ordering::SeqCst);
    }
    fn op(&self, operation: ServiceOperation) -> Result<ServiceResult, ApiError> {
        StorePort::service_operation(
            &self.store,
            operation,
            &self.connection,
            &self.gate,
            &budget(),
            None,
        )
    }
    /// Binds key `<seat>-key` to the seat.
    fn key(&self, seat: &str) {
        self.op(ServiceOperation::ReserveContinuityKey(
            ReserveContinuityKey {
                key: ContinuityKey::new(format!("{seat}-key")),
                seat: Some(SeatId::new(seat)),
                operation: OperationId::new(format!("reserve-{seat}-{}", uuid::Uuid::new_v4())),
            },
        ))
        .unwrap();
    }
    /// The closure proof of the seat's target.
    fn close(&self, seat: &str) -> Result<RetirementStart, ApiError> {
        StorePort::begin_retirement(
            &self.store,
            SeatId::new(seat),
            ClosureEvidence {
                host_boot: HostBootId::new("b"),
                epoch: 1,
                target: HostTargetId::new(seat),
                generation: 1,
            },
            &budget(),
        )
    }
    fn unretire(&self, seat: &str, operation: &str) -> Result<ServiceResult, ApiError> {
        self.op(ServiceOperation::UnretireSeat(UnretireSeat {
            seat: SeatId::new(seat),
            operation: OperationId::new(operation),
        }))
    }
    fn finalize(&self, seat: &str, operation: &str) -> Result<ServiceResult, ApiError> {
        self.op(ServiceOperation::FinalizeRetirement(FinalizeRetirement {
            seat: SeatId::new(seat),
            operation: OperationId::new(operation),
        }))
    }
    fn operator_retire(&self, seat: &str, operation: &str) -> Result<(), ApiError> {
        let mut db = self.db();
        seats::mutate_operator(
            &self.store.context,
            &mut db,
            INSTANCE,
            crate::ports::OperatorRequest::Retire(OperatorRetire {
                seat: SeatId::new(seat),
                operation: OperationId::new(operation),
            }),
            OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap(),
            None,
        )
        .map(|_| ())
    }
    fn send(&self, key: &str, recipients: &[&str], deadline: u64) -> Result<MessageId, ApiError> {
        let result = self.op(ServiceOperation::Send(ServiceSend {
            thread: ThreadId::new("t"),
            body: "body".into(),
            recipients: recipients.iter().map(|s| SeatId::new(*s)).collect(),
            deadline_millis: Some(deadline),
            operation: OperationId::new(key),
        }))?;
        let ServiceResult::MessageSent(sent) = result else {
            panic!("send answered {result:?}");
        };
        Ok(sent.summary.message)
    }
    /// Runs the message's `send_attention` job, materializing its receipts.
    fn drain(&self, message: &MessageId) {
        let job = format!("work:send:{}", message.as_str());
        let mut db = self.db();
        for _ in 0..64 {
            let status: String = db
                .query_row("SELECT status FROM work_jobs WHERE id=?1", [&job], |r| {
                    r.get(0)
                })
                .unwrap();
            if status == "complete" {
                return;
            }
            materialization::advance_work(
                &mut db,
                &job,
                crate::ports::DurableWorkAdmission::new(16).unwrap(),
                &budget(),
                self.store.context.clock(),
            )
            .unwrap();
        }
        panic!("send projection did not terminate");
    }
    fn settle_job(&self, seat: &str) {
        let job: String = self
            .db()
            .query_row("SELECT id FROM retirements WHERE seat_id=?1", [seat], |r| {
                r.get(0)
            })
            .unwrap();
        for _ in 0..64 {
            let progress = StorePort::advance_retirement(
                &self.store,
                crate::protocol::ids::RetirementJobId::new(job.clone()),
                WorkAdmission::Background,
                &budget(),
            )
            .unwrap();
            assert_eq!(progress.last_error, None);
            if progress.complete {
                return;
            }
        }
        panic!("retirement job did not finish");
    }
    fn int(&self, sql: &str) -> i64 {
        self.db().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn text(&self, sql: &str) -> String {
        self.db().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn rows(&self, table: &str, seat: &str) -> i64 {
        self.db()
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE seat_id=?1"),
                [seat],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn row_state(&self, seat: &str) -> Option<RowState> {
        row_state(&self.db(), &SeatId::new(seat)).unwrap()
    }
    fn seat_state(&self, seat: &str) -> String {
        self.db()
            .query_row("SELECT state FROM seats WHERE id=?1", [seat], |r| r.get(0))
            .unwrap()
    }
    fn receipt(&self, message: &MessageId, seat: &str) -> effective::EffectiveReceipt {
        effective::effective_receipt(&self.db(), message.as_str(), seat)
            .unwrap()
            .unwrap()
    }
    /// A standing conflict row on the key of `seat`.
    fn conflict(&self, seat: &str) {
        crate::store::continuity_keys::record_conflict(
            &self.db(),
            &crate::store::continuity_keys::ConflictFacts {
                host: crate::test_support::test_host_id(),
                target: format!("elsewhere-{seat}"),
                key: ContinuityKey::new(format!("{seat}-key")),
                kind: ContinuityConflictKind::KeyLiveElsewhere,
                seat: Some(SeatId::new(seat)),
                owner: None,
            },
            UtcMillis(100),
        )
        .unwrap();
    }
    /// The key row and its conflict rows are gone, so the key can be reserved again.
    fn assert_key_released(&self, seat: &str, route: &str) {
        let key = format!("{seat}-key");
        assert_eq!(
            crate::store::continuity_keys::lookup_key(&self.db(), &ContinuityKey::new(&key))
                .unwrap(),
            None,
            "{route}"
        );
        assert_eq!(
            self.int(&format!(
                "SELECT count(*) FROM continuity_key_conflicts WHERE key='{key}'"
            )),
            0,
            "{route}"
        );
        self.op(ServiceOperation::ReserveContinuityKey(
            ReserveContinuityKey {
                key: ContinuityKey::new(key),
                seat: None,
                operation: OperationId::new(format!("again-{route}")),
            },
        ))
        .unwrap();
    }
}

/// Kills: a key holder's close writing a job or leaving the row out; a retired read that needs the
/// overlay; a window send being accepted.
#[test]
fn closed_retirement_of_key_holder_writes_reversible_row_and_no_job() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let message = f.send("before", &["s1"], 1_000).unwrap();
    f.at(300);
    assert_eq!(f.close("s1").unwrap(), RetirementStart::Reversible);
    assert_eq!(f.row_state("s1"), Some(RowState::Reversible));
    assert_eq!(f.rows("retirements", "s1"), 0);
    assert_eq!(f.seat_state("s1"), "retired");
    assert_eq!(
        f.int("SELECT retired_at FROM seats WHERE id='s1'"),
        300,
        "the seat really retires"
    );
    assert_eq!(
        f.int("SELECT count(*) FROM occupant_bindings WHERE seat_id='s1' AND ended_at IS NULL"),
        0
    );
    // The key stays, the obligation reads retired, membership and receipts stay physically.
    assert!(
        crate::store::continuity_keys::key_of_seat(&f.db(), &SeatId::new("s1"))
            .unwrap()
            .is_some()
    );
    assert_eq!(
        f.receipt(&message, "s1").state,
        EffectiveReceiptState::RecipientRetired
    );
    assert_eq!(
        f.text("SELECT state FROM memberships WHERE seat_id='s1'"),
        "joined"
    );
    // A send to it is refused with today's error.
    assert_eq!(
        f.send("window", &["s1"], 1_000).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
}

/// Kills: reversibility leaking to a seat that holds no key.
#[test]
fn keyless_close_stays_final() {
    let f = fx(&["s1"]);
    let RetirementStart::Job(_) = f.close("s1").unwrap() else {
        panic!("a keyless close yields a job");
    };
    assert_eq!(f.row_state("s1"), None);
    assert_eq!(f.rows("retirements", "s1"), 1);
}

/// Kills: a retried proof reading as stale (the seat is retired, its target unchanged), or writing
/// a second row.
#[test]
fn retried_closure_proof_is_idempotent_on_a_reversible_row() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.close("s1").unwrap();
    assert_eq!(f.close("s1").unwrap(), RetirementStart::Reversible);
    assert_eq!(f.rows("reversible_retirements", "s1"), 1);
    assert_eq!(f.rows("retirements", "s1"), 0);
}

/// Kills: an operator retire of a live key holder going reversible, or keeping the key.
#[test]
fn operator_retire_of_a_live_key_holder_is_final_and_releases_the_key() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.conflict("s1");
    f.operator_retire("s1", "op-1").unwrap();
    assert_eq!(f.row_state("s1"), None);
    assert_eq!(f.rows("retirements", "s1"), 1);
    f.assert_key_released("s1", "operator");
}

/// One case per route that writes a `retirements` row for a key holder. Kills: a route that writes
/// the row and keeps the key. `--replace` is
/// `operator_replace_of_a_key_holder_is_final_and_releases_the_key` in `tests/store/control.rs`;
/// the live operator retire is its own test above.
#[test]
fn every_retirement_route_releases_the_key() {
    // Finalization by the service, of a reversible and of an unretired seat.
    for unretired in [false, true] {
        let f = fx(&["s1"]);
        f.key("s1");
        f.conflict("s1");
        f.close("s1").unwrap();
        if unretired {
            f.unretire("s1", "unretire").unwrap();
        }
        f.finalize("s1", "finalize").unwrap();
        assert_eq!(f.rows("retirements", "s1"), 1, "unretired={unretired}");
        f.assert_key_released("s1", "service");
    }
    // Finalization by the operator, of a reversible and of an unretired seat.
    for unretired in [false, true] {
        let f = fx(&["s1"]);
        f.key("s1");
        f.conflict("s1");
        f.close("s1").unwrap();
        if unretired {
            f.unretire("s1", "unretire").unwrap();
        }
        f.operator_retire("s1", "op").unwrap();
        assert_eq!(f.seat_state("s1"), "retired");
        assert_eq!(f.row_state("s1"), None, "unretired={unretired}");
        assert_eq!(f.rows("retirements", "s1"), 1, "unretired={unretired}");
        f.assert_key_released("s1", "operator-finalize");
    }
    // The replicated writer: an own `seat.retired` applied to a live local row.
    let f = fx(&["s1"]);
    f.key("s1");
    f.conflict("s1");
    let mut db = f.db();
    let tx = db.transaction().unwrap();
    let input = materialize::local_only(
        &tx,
        DecisionInstant {
            utc: UtcMillis(200),
            monotonic: MonoInstant(1),
        },
    )
    .unwrap();
    seats::materialize_seat_retired(
        &tx,
        &input,
        &SeatRetiredPayload {
            seat: SeatId::new("s1"),
            reason: RetireReason::Operator,
            retired_at: 200,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    drop(db);
    assert_eq!(f.rows("retirements", "s1"), 1);
    f.assert_key_released("s1", "replicated");
}

/// A shell fingerprint for `seat` (ht-ma6i.1).
fn shell_row(db: &Connection, seat: &str) {
    db.execute(
        "INSERT INTO seat_shell_fingerprints(seat_id,host_id,target_id,host_boot,incarnation,terminal_id,pid,start,session,tty,kernel_boot,recorded_at) VALUES (?1,(SELECT min(host_id) FROM herdr_hosts),?1,'b','inc','term',4242,7,4242,34817,'kb',1)",
        [seat],
    )
    .unwrap();
}

/// Kills: a fingerprint that survives un-retire, or that a reversible retirement already dropped
/// (un-retire must still have something to delete, and finalization too).
#[test]
fn row_goes_at_unretire() {
    let f = fx(&["s1"]);
    f.key("s1");
    shell_row(&f.db(), "s1");
    f.close("s1").unwrap();
    assert_eq!(
        f.rows("seat_shell_fingerprints", "s1"),
        1,
        "reversible keeps it"
    );
    f.unretire("s1", "unretire").unwrap();
    assert_eq!(f.rows("seat_shell_fingerprints", "s1"), 0);
}

/// Kills: a fingerprint that survives the deferred job of a reversible seat's finalization.
#[test]
fn row_goes_at_finalization_of_a_reversible_seat() {
    let f = fx(&["s1"]);
    f.key("s1");
    shell_row(&f.db(), "s1");
    f.close("s1").unwrap();
    f.finalize("s1", "finalize").unwrap();
    assert_eq!(f.rows("seat_shell_fingerprints", "s1"), 0);
}

/// Kills: a rival query that counts a seat reading retired, which would strand the live seat a
/// replacement left in the same shell while the old one waits on its service.
#[test]
fn reversible_seat_row_is_no_rival() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    shell_row(&f.db(), "s1");
    let db = f.db();
    let shell = crate::identity::shell_continuity::ShellFingerprint {
        pid: 4242,
        start: 7,
        session: 4242,
        tty: 34817,
        kernel_boot: "kb".into(),
    };
    let rival = |seat: &str| seats::shell_rival_exists(&db, &SeatId::new(seat), &shell).unwrap();
    assert!(rival("s2"), "a live seat's row is a rival");
    f.close("s1").unwrap();
    assert_eq!(f.rows("seat_shell_fingerprints", "s1"), 1);
    assert!(!rival("s2"), "a reversibly retired seat's row is not");
}

/// Kills: a seat that comes back placed, bound or still carrying its retirement or proof; no new
/// unavailability episode; a generation that does not move.
#[test]
fn unretire_clears_retired_fields_and_proof_and_opens_an_episode() {
    let f = fx(&["s1"]);
    f.key("s1");
    let episode = f.int("SELECT unavailability_episode FROM seats WHERE id='s1'");
    f.close("s1").unwrap();
    let generation = f.int("SELECT generation FROM seats WHERE id='s1'");
    f.at(500);
    let ServiceResult::SeatUnretired(result) = f.unretire("s1", "u1").unwrap() else {
        panic!("not an unretire result");
    };
    assert_eq!(result.seat, SeatId::new("s1"));
    assert_eq!(result.generation, (generation + 1) as u64);
    type Columns = (
        String,
        String,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
    );
    let columns: Columns = f
        .db()
        .query_row(
            "SELECT state,unresolved_reason,target_id,retired_at,retired_seq,structural_terminal_id FROM seats WHERE id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .unwrap();
    assert_eq!(
        columns,
        ("unresolved".into(), "other".into(), None, None, None, None)
    );
    assert_eq!(
        f.int("SELECT unavailability_open FROM seats WHERE id='s1'"),
        1
    );
    assert!(f.int("SELECT unavailability_episode FROM seats WHERE id='s1'") > episode);
    assert_eq!(
        f.int("SELECT count(*) FROM occupant_bindings WHERE seat_id='s1' AND ended_at IS NULL"),
        0
    );
    assert_eq!(f.row_state("s1"), Some(RowState::Unretired));
    assert_eq!(
        f.int("SELECT unretired_at FROM reversible_retirements WHERE seat_id='s1'"),
        500
    );
    assert_eq!(
        f.text(
            "SELECT kind FROM allocation_decisions WHERE seat_id='s1' ORDER BY ordinal DESC LIMIT 1"
        ),
        "service_unretire"
    );
    assert_eq!(
        f.text("SELECT target_id FROM allocation_decisions WHERE seat_id='s1' ORDER BY ordinal DESC LIMIT 1"),
        "s1",
        "at the row's closure target"
    );
    // Replay: the journaled first result; a new key on the now unretired seat is refused.
    assert_eq!(
        f.unretire("s1", "u1").unwrap(),
        ServiceResult::SeatUnretired(result)
    );
    assert_eq!(
        f.unretire("s1", "u2").unwrap_err().code,
        ErrorCode::NotFound
    );
}

/// Kills: an un-retire by a service that does not own the key; a seat with no row being touched.
#[test]
fn unretire_is_for_the_owner_of_a_reversible_seat() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    f.close("s1").unwrap();
    {
        let mut db = f.db();
        let tx = db.transaction().unwrap();
        let error = unretire(
            &tx,
            &f.store.context,
            INSTANCE,
            &ServiceAuthorId::new("other-author"),
            &UnretireSeat {
                seat: SeatId::new("s1"),
                operation: OperationId::new("u"),
            },
            DecisionInstant {
                utc: UtcMillis(200),
                monotonic: MonoInstant(1),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unauthorized);
        assert_eq!(
            finalize(
                &tx,
                &f.store.context,
                INSTANCE,
                FinalizeBy::Service(&ServiceAuthorId::new("other-author")),
                &SeatId::new("s1"),
                DecisionInstant {
                    utc: UtcMillis(200),
                    monotonic: MonoInstant(1),
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::Unauthorized
        );
    }
    assert_eq!(f.row_state("s1"), Some(RowState::Reversible));
    // A keyless seat has no owner; a live key holder has no retirement.
    assert_eq!(
        f.unretire("s2", "u2").unwrap_err().code,
        ErrorCode::Unauthorized
    );
    f.key("s2");
    assert_eq!(
        f.unretire("s2", "u3").unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(f.seat_state("s2"), "resolved");
    assert_eq!(
        f.finalize("s2", "f3").unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(f.seat_state("s2"), "resolved");
}

/// Kills: a deadline that does not move, moves by the wrong amount or misses a table; a logical
/// receipt keeping its projection; `receipt_due_sparse` left behind; another seat's obligation or
/// the seat's own outgoing invitation moving.
#[test]
fn deadlines_keep_remaining_time_including_logical_receipts_and_invitations() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let logical = f.send("logical", &["s1"], 1_000).unwrap();
    let materialized = f.send("materialized", &["s1", "s2"], 1_000).unwrap();
    f.drain(&materialized);
    let physical = f.send("physical", &["s2"], 1_000).unwrap();
    {
        let db = f.db();
        db.execute(
            "INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,deadline_at) VALUES (?1,'t','s1','pending',1000,1100)",
            [physical.as_str()],
        )
        .unwrap();
        for (id, invitee, inviter) in [("owed", "s1", "s2"), ("outgoing", "s2", "s1")] {
            db.execute(
                "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node,inviter_seat_id) VALUES (?1,'t',?2,1,'pending',1,0,1000,1100,(SELECT id FROM local_node),?3)",
                params![id, invitee, inviter],
            )
            .unwrap();
        }
    }
    let invitation = |id: &str| -> i64 {
        f.db()
            .query_row(
                "SELECT deadline_at FROM invitations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    let stored = |message: &MessageId, seat: &str| -> i64 {
        f.db()
            .query_row(
                "SELECT deadline_at FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
                params![message.as_str(), seat],
                |r| r.get(0),
            )
            .unwrap()
    };
    let legacy = || f.int("SELECT deadline_at FROM receipts WHERE seat_id='s1'");
    // Each class against its own pre-retire value.
    let materialized_before = stored(&materialized, "s1");
    let legacy_before = legacy();
    let owed_before = invitation("owed");
    let logical_before = key_deadline(&f, &logical, "s1").1;
    f.at(300);
    f.close("s1").unwrap();
    f.at(800);
    f.unretire("s1", "u").unwrap();
    assert_eq!(stored(&materialized, "s1"), materialized_before + 500);
    assert_eq!(legacy(), legacy_before + 500);
    assert_eq!(invitation("owed"), owed_before + 500);
    assert_eq!(stored(&logical, "s1"), logical_before + 500);
    // Frozen for 500 ms: 800 ms were left at the cutover and at the un-retire.
    assert_eq!(stored(&logical, "s1"), 1600);
    assert_eq!(stored(&materialized, "s1"), 1600);
    assert_eq!(f.receipt(&logical, "s1").deadline_at, Some(1600));
    assert_eq!(
        f.receipt(&logical, "s1").state,
        EffectiveReceiptState::Pending
    );
    assert_eq!(f.receipt(&materialized, "s1").deadline_at, Some(1600));
    assert_eq!(
        f.int("SELECT deadline_at FROM receipts WHERE seat_id='s1'"),
        1600
    );
    assert_eq!(invitation("owed"), 1600);
    assert_eq!(
        invitation("outgoing"),
        1100,
        "its own request keeps its deadline"
    );
    assert_eq!(stored(&materialized, "s2"), 1100, "another seat's stays");
    // The sparse due index follows.
    assert_eq!(
        f.int("SELECT count(*) FROM receipt_due_sparse WHERE seat_id='s1' AND deadline_at=1600"),
        2
    );
    assert_eq!(
        f.int("SELECT count(*) FROM receipt_due_sparse WHERE seat_id='s1' AND deadline_at<>1600"),
        0
    );
}

/// Kills: the bound missing from any of the shift's routes, so a replayed `seat.unretired` moves an
/// obligation created after the un-retire, or a bound that also stops one created exactly at it.
/// Only this fixture holds an unprojected manifest, which the replay scenario cannot reach.
#[test]
fn shift_moves_only_what_was_created_by_the_bound() {
    let f = fx(&["s1"]);
    f.at(100);
    let logical_before = f.send("logical-before", &["s1"], 1_000).unwrap();
    let materialized_before = f.send("materialized-before", &["s1"], 1_000).unwrap();
    f.drain(&materialized_before);
    f.at(500);
    let logical_at = f.send("logical-at", &["s1"], 1_000).unwrap();
    f.at(900);
    let logical_after = f.send("logical-after", &["s1"], 1_000).unwrap();
    let materialized_after = f.send("materialized-after", &["s1"], 1_000).unwrap();
    f.drain(&materialized_after);
    {
        let db = f.db();
        for (id, created_at) in [("before", 100), ("at", 500), ("after", 900)] {
            db.execute(
                "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node) VALUES (?1,'t','s1',?2,'pending',1,?2,1000,?2+1000,(SELECT id FROM local_node))",
                params![id, created_at],
            )
            .unwrap();
        }
        // The legacy table's route: its message's time is `messages.decision_at` too.
        for (message, deadline) in [(&materialized_before, 1_100), (&materialized_after, 1_900)] {
            db.execute(
                "INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,deadline_at) VALUES (?1,'t','s1','pending',1000,?2)",
                params![message.as_str(), deadline],
            )
            .unwrap();
        }
        // Frozen from 450 to 500: what was created by 450 moves by 50, what was created at 500 by
        // nothing (but still gets its stored row), what came later stays.
        super::shift_pending_deadlines(&db, &SeatId::new("s1"), 450, 500).unwrap();
    }
    let int = |sql: String| f.int(&sql);
    let stored = |message: &MessageId| {
        int(format!(
            "SELECT deadline_at FROM receipt_state WHERE message_id='{}' AND seat_id='s1'",
            message.as_str()
        ))
    };
    assert_eq!(stored(&logical_before), 1_150);
    assert_eq!(stored(&logical_at), 1_500, "created exactly at the bound");
    assert_eq!(
        int(format!(
            "SELECT count(*) FROM receipt_state WHERE message_id='{}'",
            logical_after.as_str()
        )),
        0,
        "a later manifest stays logical"
    );
    assert_eq!(f.receipt(&logical_after, "s1").deadline_at, Some(1_900));
    assert_eq!(stored(&materialized_before), 1_150);
    assert_eq!(stored(&materialized_after), 1_900);
    let legacy = |message: &MessageId| {
        int(format!(
            "SELECT deadline_at FROM receipts WHERE message_id='{}'",
            message.as_str()
        ))
    };
    assert_eq!(legacy(&materialized_before), 1_150);
    assert_eq!(legacy(&materialized_after), 1_900);
    let invitation = |id: &str| {
        int(format!(
            "SELECT deadline_at FROM invitations WHERE id='{id}'"
        ))
    };
    assert_eq!(invitation("before"), 1_150);
    assert_eq!(invitation("at"), 1_500);
    assert_eq!(invitation("after"), 1_900);
}

/// Kills: an obligation created inside the window moving by the whole window instead of the time
/// it spent frozen (`unretired_at - created_at`), on the receipt and the invitation routes; one
/// created before the retirement moving by less than the whole window; a soft poke measured from
/// any deadline other than the shifted one.
#[test]
fn a_mid_window_obligation_shifts_by_its_own_frozen_time() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let before = f.send("before", &["s1"], 1_000).unwrap();
    f.at(300);
    f.close("s1").unwrap();
    // At 500, mid-window: a window message the home node holds pending for the frozen seat (sends to
    // a frozen seat are refused here, so `s2`'s receipt is copied to it) and a window invitation.
    f.at(500);
    let mid = f.send("mid", &["s2"], 1_000).unwrap();
    {
        let db = f.db();
        db.execute(
            "INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,ack_required) SELECT pr.preparation_id,'s1',pr.thread_id,pr.receipt_ordinal+1,pr.frozen_duration_ms,1,1 FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 AND pr.seat_id='s2'",
            [mid.as_str()],
        )
        .unwrap();
        for (id, created_at) in [("i-before", 100), ("i-mid", 500)] {
            db.execute(
                "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node) VALUES (?1,'t','s1',?2,'pending',1,?2,1000,?2+1000,(SELECT id FROM local_node))",
                params![id, created_at],
            )
            .unwrap();
        }
    }
    f.at(800);
    f.unretire("s1", "u").unwrap();
    let stored = |message: &MessageId| -> (i64, i64) {
        f.db()
            .query_row(
                "SELECT available_at,deadline_at FROM receipt_state WHERE message_id=?1 AND seat_id='s1'",
                [message.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    let invitation = |id: &str| {
        f.int(&format!(
            "SELECT deadline_at FROM invitations WHERE id='{id}'"
        ))
    };
    // Frozen 300 to 800: before the retirement by 500, from 500 by 300.
    assert_eq!(stored(&before), (100, 1_600));
    assert_eq!(invitation("i-before"), 1_600);
    assert_eq!(stored(&mid), (500, 1_800), "not 2_000: the whole window");
    assert_eq!(invitation("i-mid"), 1_800);
    assert_eq!(f.receipt(&mid, "s1").deadline_at, Some(1_800));
    assert_eq!(
        f.int(&format!(
            "SELECT deadline_at FROM receipt_due_sparse WHERE seat_id='s1' AND message_id='{}'",
            mid.as_str()
        )),
        1_800
    );

    // The soft poke: the seat is reattached so it can be poked; the mid-window receipt becomes a
    // candidate at the soft point of its shifted deadline, never earlier.
    f.db()
        .execute(
            "UPDATE seats SET state='resolved',unresolved_reason=NULL,target_id='s1' WHERE id='s1'",
            [],
        )
        .unwrap();
    let settings = crate::protocol::summary::SummarySettings {
        soft_fraction: 0.5,
        ..Default::default()
    };
    let soft = crate::notification::policy::soft_point(1_800, 1_800 - 500, 0.5);
    assert_eq!(soft, 1_150);
    let poked = |now: i64| -> bool {
        poke::due_pokes_for_seat(&f.db(), &SeatId::new("s1"), now, &settings)
            .unwrap()
            .is_some_and(|due| due.receipts.iter().any(|r| r.message == mid))
    };
    assert!(
        !poked(soft - 1),
        "no soft poke before the shifted deadline's soft point"
    );
    assert!(poked(soft));
}

/// Kills: the home node shifting an obligation peers leave, under origin clock skew: home
/// `UnretireSeat` bounds the shift by the `unretired_at` it publishes, as `unretire_by_fact` does
/// for a peer and an own-log replay.
#[test]
fn home_unretire_bounds_the_shift_by_its_unretired_at() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let normal = f.send("normal", &["s1"], 1_000).unwrap();
    // A clock ahead of the un-retire: the message reads as decided after it.
    f.at(5_000);
    let skewed = f.send("skewed", &["s1"], 1_000).unwrap();
    let skewed_before = f.receipt(&skewed, "s1").deadline_at;
    assert_eq!(skewed_before, Some(6_000));
    let normal_before = f.receipt(&normal, "s1").deadline_at.unwrap();
    f.at(300);
    f.close("s1").unwrap();
    f.at(800);
    f.unretire("s1", "u").unwrap();
    let stored = |message: &MessageId| -> Option<i64> {
        f.db()
            .query_row(
                "SELECT deadline_at FROM receipt_state WHERE message_id=?1 AND seat_id='s1'",
                [message.as_str()],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
    };
    assert_eq!(stored(&normal), Some(normal_before + 500));
    assert_eq!(
        stored(&skewed),
        None,
        "decided after unretired_at: not shifted"
    );
    assert_eq!(f.receipt(&skewed, "s1").deadline_at, skewed_before);
}

/// `(available_at, key deadline)` of the shared projection, with its pending deadline checked equal
/// to the key deadline (they agree for a pending receipt with no stored deadline).
fn key_deadline(f: &Fx, message: &MessageId, seat: &str) -> (Option<i64>, i64) {
    let (receipt, key) =
        effective::effective_receipt_with_key_deadline(&f.db(), message.as_str(), seat)
            .unwrap()
            .unwrap();
    assert_eq!(receipt.state, EffectiveReceiptState::Pending);
    assert_eq!(receipt.deadline_at, key);
    (receipt.available_at, key.unwrap())
}

/// Copies `s2`'s logical receipt of `message` to `s1` as not eligible at the snapshot, and records
/// `s1` available at 250 right after the manifest. Returns the manifest's decision sequence.
fn not_yet_eligible(f: &Fx, message: &MessageId) -> i64 {
    let db = f.db();
    db.execute(
        "INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,ack_required) SELECT pr.preparation_id,'s1',pr.thread_id,pr.receipt_ordinal+1,pr.frozen_duration_ms,0,1 FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 AND pr.seat_id='s2'",
        [message.as_str()],
    )
    .unwrap();
    let send_seq: i64 = db
        .query_row(
            "SELECT decision_seq FROM send_manifests WHERE message_id=?1",
            [message.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    db.execute(
        "INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('s1',?1,250,1,'cooperative_top_level')",
        [send_seq + 1],
    )
    .unwrap();
    send_seq
}

/// Freezes `s1` from 300 to 800 and checks each manifest's stored receipt is its pre-retire
/// projection with the deadline moved by the 500 ms frozen.
fn assert_shifted_from_projection(f: &Fx, manifests: &[(&MessageId, (Option<i64>, i64))]) {
    f.at(300);
    f.close("s1").unwrap();
    f.at(800);
    f.unretire("s1", "u").unwrap();
    for (message, (available_at, deadline)) in manifests {
        let stored: (Option<i64>, i64) = f
            .db()
            .query_row(
                "SELECT available_at,deadline_at FROM receipt_state WHERE message_id=?1 AND seat_id='s1'",
                [message.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored, (*available_at, deadline + 500), "{message:?}");
        let receipt = f.receipt(message, "s1");
        assert_eq!(receipt.deadline_at, Some(deadline + 500));
        assert_eq!(receipt.state, EffectiveReceiptState::Pending);
    }
}

/// Kills: a wrong `available_at` or deadline on any branch of the SQL spelling of the logical
/// projection: eligible at the snapshot, not yet eligible (availability from `seat_availability`)
/// and a manifest under a `receipt_send_time_cutovers` row.
#[test]
fn logical_receipt_shift_matches_the_shared_projection_in_every_form() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let eligible = f.send("eligible", &["s1"], 1_000).unwrap();
    let later = f.send("later", &["s2"], 1_000).unwrap();
    not_yet_eligible(&f, &later);
    let eligible_before = key_deadline(&f, &eligible, "s1");
    let later_before = key_deadline(&f, &later, "s1");
    assert_eq!(eligible_before, (Some(100), 1_100));
    assert_eq!(later_before, (Some(250), 1_100), "deadline from send time");
    assert_shifted_from_projection(&f, &[(&eligible, eligible_before), (&later, later_before)]);

    // The cutover covers every manifest of its instance, so this form has its own store.
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let historical = f.send("historical", &["s2"], 1_000).unwrap();
    let send_seq = not_yet_eligible(&f, &historical);
    f.db()
        .execute(
            "INSERT INTO receipt_send_time_cutovers(instance_id,historical_decision_seq) SELECT instance_id,?2 FROM send_manifests WHERE message_id=?1",
            params![historical.as_str(), send_seq + 1],
        )
        .unwrap();
    let historical_before = key_deadline(&f, &historical, "s1");
    assert_eq!(
        historical_before,
        (Some(250), 1_250),
        "the clock started at the recorded availability"
    );
    assert_shifted_from_projection(&f, &[(&historical, historical_before)]);
}

/// Kills: a frozen seat's scan row eating the page, so another seat's due row waits behind it; an
/// overdue warning or a soft poke for what a frozen seat owes.
#[test]
fn frozen_rows_do_not_starve_another_seats_due_row() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let frozen = f.send("frozen", &["s1"], 50).unwrap();
    f.drain(&frozen);
    let live = f.send("live", &["s2"], 50).unwrap();
    f.drain(&live);
    let mut db = f.db();
    for (id, seat, episode) in [("i-frozen", "s1", 1), ("i-live", "s2", 1)] {
        db.execute(
            "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node) VALUES (?1,'t',?2,?3,'pending',1,0,50,50,(SELECT id FROM local_node))",
            params![id, seat, episode],
        )
        .unwrap();
    }
    f.at(120);
    f.close("s1").unwrap();
    f.at(140);
    // Past the soft point: only the live seat's row is due.
    let settings = crate::protocol::summary::SummarySettings {
        soft_fraction: 0.5,
        ..Default::default()
    };
    assert!(
        poke::due_pokes_for_seat(&db, &SeatId::new("s1"), 140, &settings)
            .unwrap()
            .is_none()
    );
    f.at(1_000);
    // A page of one: the frozen row, older in every order, must not fill it.
    let mut cursor = receipts::ReceiptDueCursor::default();
    assert_eq!(
        receipts::scan_due(&f.store.context, &mut db, 1, &mut cursor)
            .unwrap()
            .warnings,
        1
    );
    let warned = |message: &MessageId, seat: &str| -> bool {
        f.db().query_row(
            "SELECT warning_message_id IS NOT NULL FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
            params![message.as_str(), seat],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(warned(&live, "s2"));
    assert!(!warned(&frozen, "s1"));
    let page = invitation_due::select_invitation_due_candidates(&db, None, 1).unwrap();
    invitation_due::apply_invitation_due_candidates(&f.store.context, &mut db, page).unwrap();
    let invitation_warned = |id: &str| -> bool {
        f.db()
            .query_row(
                "SELECT warning_message_id IS NOT NULL FROM invitations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert!(invitation_warned("i-live"));
    assert!(!invitation_warned("i-frozen"));
    // After the un-retire its row is due again by the same scans.
    f.unretire("s1", "u").unwrap();
    f.at(2_000);
    let mut cursor = receipts::ReceiptDueCursor::default();
    receipts::scan_due(&f.store.context, &mut db, 10, &mut cursor).unwrap();
    assert!(warned(&frozen, "s1"));
}

/// Kills: a requirement or membership needing re-acceptance after the window (fence, un-retire,
/// read).
#[test]
fn memberships_and_episodes_are_effective_with_no_reacceptance() {
    let f = fx(&["s1"]);
    f.key("s1");
    let thread = ThreadId::new("t");
    let seat = SeatId::new("s1");
    let result = f
        .op(ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: seat.clone(),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(1_000),
            operation: OperationId::new("require"),
        }))
        .unwrap();
    let ServiceResult::Invitation(invitation) = result else {
        panic!("invite answered {result:?}");
    };
    let requirement = invitation.requirement.unwrap();
    let read = |f: &Fx| {
        let db = f.db();
        (
            service_substrate::effective_membership(&db, &thread, &seat)
                .unwrap()
                .voluntary,
            service_substrate::current_requirement(&db, &thread, &seat)
                .unwrap()
                .map(|current| current.requirement),
        )
    };
    f.close("s1").unwrap();
    assert_eq!(read(&f).0, VoluntaryMembershipState::Retired);
    f.unretire("s1", "u").unwrap();
    assert_eq!(
        read(&f),
        (
            VoluntaryMembershipState::Joined,
            Some(requirement.requirement)
        )
    );
    assert_eq!(
        f.text("SELECT state FROM requirement_episodes WHERE seat_id='s1'"),
        "pending"
    );
}

/// Kills: a waiting seat holding the host's restore hold.
#[test]
fn unretired_seat_does_not_block_hold_lift() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.close("s1").unwrap();
    f.unretire("s1", "u").unwrap();
    let mut db = f.db();
    db.execute_batch(
        "UPDATE herdr_hosts SET recovery_boot='r',recovery_epoch=1,reconciled_boot='r',reconciled_epoch=1,baseline_hold_unclaimed=1",
    )
    .unwrap();
    let host = crate::test_support::test_host_id();
    let tx = db.transaction().unwrap();
    // The same seat without its row blocks: the fixture proves the lift can refuse.
    tx.execute("DELETE FROM reversible_retirements WHERE seat_id='s1'", [])
        .unwrap();
    assert!(!seats::lift_baseline_hold_if_clear(&tx, INSTANCE, host, UtcMillis(1)).unwrap());
    tx.execute(
        "INSERT INTO reversible_retirements(seat_id,state,cutover_at,reason,unretired_at) VALUES ('s1','unretired',1,'closed',2)",
        [],
    )
    .unwrap();
    assert!(seats::lift_baseline_hold_if_clear(&tx, INSTANCE, host, UtcMillis(1)).unwrap());
}

/// Kills: a repair line naming a seat with no pane, or health counting it as a blocker.
#[test]
fn unretired_seat_gets_no_rebind_suggestion() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.close("s1").unwrap();
    let summary = |f: &Fx| {
        queries::unresolved_seat_summary(&f.store.context, INSTANCE, &budget())
            .unwrap()
            .count
    };
    f.unretire("s1", "u").unwrap();
    let db = f.db();
    let host = crate::test_support::test_host_id();
    let tx = db.unchecked_transaction().unwrap();
    let refusal =
        crate::store::seats::resolution::resolution_recovery_refusal(&tx, host, "pane", "held")
            .unwrap();
    assert!(!refusal.detail.contains("s1"), "{}", refusal.detail);
    drop(tx);
    assert_eq!(summary(&f), 0);
    // The same seat without the row is a blocker: both reads see it.
    db.execute("DELETE FROM reversible_retirements WHERE seat_id='s1'", [])
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let refusal =
        crate::store::seats::resolution::resolution_recovery_refusal(&tx, host, "pane", "held")
            .unwrap();
    assert!(refusal.detail.contains("s1"), "{}", refusal.detail);
    drop(tx);
    assert_eq!(summary(&f), 1);
}

/// Kills: the job re-reading a moved cutover; evidence lost on the way to the job; the job leaving
/// obligations pending.
#[test]
fn the_job_settles_obligations_after_finalize() {
    let f = fx(&["s1"]);
    f.key("s1");
    let logical = f.send("logical", &["s1"], 1_000).unwrap();
    let materialized = f.send("materialized", &["s1"], 1_000).unwrap();
    f.drain(&materialized);
    f.at(300);
    f.close("s1").unwrap();
    f.db()
        .execute(
            "UPDATE reversible_retirements SET closure_evidence='restored_pane_closed',absence_evidence='restored_target_absent' WHERE seat_id='s1'",
            [],
        )
        .unwrap();
    f.at(900);
    assert_eq!(
        f.finalize("s1", "f").unwrap(),
        ServiceResult::RetirementFinalized(SeatId::new("s1"))
    );
    assert_eq!(f.row_state("s1"), None);
    assert_eq!(
        f.int("SELECT cutover_at FROM retirements WHERE seat_id='s1'"),
        300,
        "the original cutover"
    );
    assert_eq!(f.int("SELECT retired_at FROM seats WHERE id='s1'"), 300);
    assert_eq!(
        f.text("SELECT closure_evidence FROM retirements WHERE seat_id='s1'"),
        "restored_pane_closed"
    );
    assert_eq!(
        f.text("SELECT absence_evidence FROM retirements WHERE seat_id='s1'"),
        "restored_target_absent"
    );
    assert_eq!(
        f.text("SELECT closure_target FROM retirements WHERE seat_id='s1'"),
        "s1"
    );
    assert_eq!(
        f.text(
            "SELECT kind FROM allocation_decisions WHERE seat_id='s1' ORDER BY ordinal DESC LIMIT 1"
        ),
        "service_finalize"
    );
    f.settle_job("s1");
    for message in [&logical, &materialized] {
        let (state, retired_at): (String, i64) = f
            .db()
            .query_row(
                "SELECT state,retired_at FROM receipt_state WHERE message_id=?1 AND seat_id='s1'",
                [message.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((state.as_str(), retired_at), ("recipient_retired", 300));
    }
    assert_eq!(
        f.text("SELECT state FROM memberships WHERE seat_id='s1'"),
        "retired"
    );
}

/// Kills: a fact applied during the window staying physically pending because the job's
/// `send_seq < retired_seq` fence still reads the fence's sequence (fence, apply, finalize, read
/// physical state).
#[test]
fn finalization_advances_retired_seq() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    f.close("s1").unwrap();
    let fenced = f.int("SELECT retired_seq FROM seats WHERE id='s1'");
    // A window fact: a manifest published after the fence with the frozen seat in its audience.
    let window = f.send("window", &["s2"], 1_000).unwrap();
    f.db()
        .execute(
            "INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,ack_required) SELECT pr.preparation_id,'s1',pr.thread_id,pr.receipt_ordinal+1,pr.frozen_duration_ms,1,1 FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 AND pr.seat_id='s2'",
            [window.as_str()],
        )
        .unwrap();
    let send_seq = f.int(&format!(
        "SELECT decision_seq FROM send_manifests WHERE message_id='{}'",
        window.as_str()
    ));
    assert!(send_seq > fenced);
    f.finalize("s1", "f").unwrap();
    assert!(f.int("SELECT retired_seq FROM seats WHERE id='s1'") > send_seq);
    f.settle_job("s1");
    assert_eq!(
        f.text(&format!(
            "SELECT state FROM receipt_state WHERE seat_id='s1' AND message_id='{}'",
            window.as_str()
        )),
        "recipient_retired"
    );
}

/// Kills: an unretired seat finalizing into another reversible row (it still holds its key when the
/// disposition runs), or being refused.
#[test]
fn finalize_of_an_unretired_seat_retires_finally() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.at(300);
    f.close("s1").unwrap();
    f.at(800);
    f.unretire("s1", "u").unwrap();
    f.at(900);
    f.finalize("s1", "f").unwrap();
    assert_eq!(f.seat_state("s1"), "retired");
    assert_eq!(f.row_state("s1"), None);
    assert_eq!(
        f.int("SELECT cutover_at FROM retirements WHERE seat_id='s1'"),
        900,
        "an ordinary retirement at this decision"
    );
    assert_eq!(
        f.text(
            "SELECT kind FROM allocation_decisions WHERE seat_id='s1' ORDER BY ordinal DESC LIMIT 1"
        ),
        "service_finalize"
    );
    f.settle_job("s1");
    assert_eq!(
        f.text("SELECT state FROM memberships WHERE seat_id='s1'"),
        "retired"
    );
    // The seat is final now: nothing is left to undo, and the operator's retire finds no live seat.
    assert_eq!(
        f.unretire("s1", "u2").unwrap_err().code,
        ErrorCode::Unauthorized
    );
    assert_eq!(
        f.operator_retire("s1", "op").unwrap_err().code,
        ErrorCode::NotFound
    );
}

/// Kills: a stale row surviving a rebind, so the next close collides or reads as a past episode.
#[test]
fn close_unretire_reattach_close_is_reversible_twice() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.close("s1").unwrap();
    f.unretire("s1", "u1").unwrap();
    let host = crate::test_support::test_host_id();
    {
        let mut db = f.db();
        let tx = db.transaction().unwrap();
        seats::operator::rebind_unresolved_seat(
            &tx,
            INSTANCE,
            host,
            &SeatId::new("s1"),
            "s1",
            1,
            UtcMillis(600),
        )
        .unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(f.row_state("s1"), None, "a rebind ends the window");
    assert_eq!(f.seat_state("s1"), "resolved");
    f.at(700);
    assert_eq!(f.close("s1").unwrap(), RetirementStart::Reversible);
    assert_eq!(f.row_state("s1"), Some(RowState::Reversible));
    assert_eq!(
        f.int("SELECT cutover_at FROM reversible_retirements WHERE seat_id='s1'"),
        700
    );
    f.at(800);
    f.unretire("s1", "u2").unwrap();
    assert_eq!(f.row_state("s1"), Some(RowState::Unretired));
}

/// Kills: a seat that stays refused after un-retire, or a send that gets no unavailable warning.
#[test]
fn send_after_unretire_succeeds_with_the_unavailable_warning() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.close("s1").unwrap();
    f.unretire("s1", "u").unwrap();
    let message = f.send("after", &["s1"], 1_000).unwrap();
    f.drain(&message);
    assert_eq!(
        f.receipt(&message, "s1").state,
        EffectiveReceiptState::Pending
    );
    let episode = f.int("SELECT unavailability_episode FROM seats WHERE id='s1'");
    assert_eq!(
        f.int(&format!(
            "SELECT count(*) FROM warning_conditions WHERE condition_kind='unavailable' AND affected_seat_id='s1' AND episode={episode}"
        )),
        1
    );
}

/// Kills: a retirement-driven revision bump that the way back forgets, leaving a cached directory.
#[test]
fn unretire_rebumps_directory_and_member_revisions() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.close("s1").unwrap();
    let revision = |key: &str| -> i64 {
        f.db()
            .query_row(
                "SELECT revision FROM filter_revisions WHERE scope_kind='directory' AND scope_key=?1",
                [key],
                |r| r.get(0),
            )
            .unwrap()
    };
    let before = (revision("all"), revision("member:s1"));
    f.unretire("s1", "u").unwrap();
    assert!(revision("all") > before.0);
    assert!(revision("member:s1") > before.1);
}

/// Spec 4.3 archival: a held-open seat is an active member on its home node and a live member on a
/// peer row (fence, un-retire, read). Kills: a quiet thread archiving under a seat that can return.
#[test]
fn archival_counts_a_reversible_or_unretired_seat() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.close("s1").unwrap();
    let active =
        |f: &Fx| archival::member_active_for_archival(&f.db(), "s1", UtcMillis(1)).unwrap();
    assert!(active(&f), "reversible");
    f.unretire("s1", "u").unwrap();
    assert!(active(&f), "unretired");
    f.finalize("s1", "f").unwrap();
    assert!(!active(&f), "final: the row is gone");

    // A peer row: a remote seat retired here, member of a thread here.
    let f = fx(&[]);
    let db = f.db();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,published,retired_at,retired_seq) VALUES ('peer',?1,'retired','native',1,0,'00000000000000000000000000000001',0,'agent',1,5,1)",
        [INSTANCE],
    )
    .unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES ('t','peer','joined','joined',1)",
        [],
    )
    .unwrap();
    assert!(!archival::has_live_remote_member(&db, "t").unwrap());
    for state in ["reversible", "unretired"] {
        db.execute(
            "INSERT INTO reversible_retirements(seat_id,state,cutover_at,reason,unretired_at) VALUES ('peer',?1,5,'closed',CASE WHEN ?1='unretired' THEN 6 END)",
            [state],
        )
        .unwrap();
        assert!(
            archival::has_live_remote_member(&db, "t").unwrap(),
            "{state}"
        );
        db.execute("DELETE FROM reversible_retirements", [])
            .unwrap();
    }
    assert!(!archival::has_live_remote_member(&db, "t").unwrap());
}

/// Spec 4.3: a frozen seat is still an origin, so its own invitation's overdue and clear warnings
/// have a recipient row, visible on reattach (fence, apply, un-retire, read). Kills: the warning
/// lost in the window; a finally retired origin keeping its row.
#[test]
fn invitation_warnings_reach_a_frozen_origin() {
    let run = |keyed: bool| -> (i64, i64) {
        let f = fx(&["s1", "s2"]);
        if keyed {
            f.key("s1");
        }
        let mut db = f.db();
        db.execute(
            "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node,inviter_seat_id,confirmation) VALUES ('out','t','s2',1,'pending',1,0,50,150,(SELECT id FROM local_node),'s1','require_ack')",
            [],
        )
        .unwrap();
        f.close("s1").unwrap();
        f.at(1_000);
        invitation_due::scan_invitation_due_batch(&f.store.context, &mut db, None, 10).unwrap();
        let warning: String = db
            .query_row(
                "SELECT warning_message_id FROM invitations WHERE id='out'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        db.execute(
            "UPDATE invitations SET state='recipient_retired',retired_at=1 WHERE id='out'",
            [],
        )
        .unwrap();
        let tx = db.transaction().unwrap();
        schema::clear_warning_condition_for_invitation(&tx, "out", UtcMillis(1_100)).unwrap();
        tx.commit().unwrap();
        if keyed {
            f.unretire("s1", "u").unwrap();
        }
        let open: i64 = db
            .query_row(
                "SELECT count(*) FROM warning_recipients WHERE seat_id='s1' AND warning_id=?1",
                [&warning],
                |r| r.get(0),
            )
            .unwrap();
        let clear: i64 = db
            .query_row(
                "SELECT count(*) FROM warning_recipients r JOIN warning_conditions c ON c.clear_warning_id=r.warning_id WHERE r.seat_id='s1' AND c.open_warning_id=?1",
                [&warning],
                |r| r.get(0),
            )
            .unwrap();
        (open, clear)
    };
    assert_eq!(run(true), (1, 1), "a frozen origin keeps both rows");
    assert_eq!(run(false), (0, 0), "a final retirement keeps neither");
}

/// Un-retire writes replicated tables (the seat, the deadlines) through the replicated scope: legal
/// under the armed guard.
#[test]
fn unretire_is_legal_under_the_armed_write_guard() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.close("s1").unwrap();
    let mut db = f.db();
    #[cfg(feature = "test-support")]
    let _guard = crate::test_support::write_guard::arm(&db);
    let tx = db.transaction().unwrap();
    unretire(
        &tx,
        &f.store.context,
        INSTANCE,
        &service_substrate::ensure_reserved_author(&tx, INSTANCE, UtcMillis(0)).unwrap(),
        &UnretireSeat {
            seat: SeatId::new("s1"),
            operation: OperationId::new("u"),
        },
        DecisionInstant {
            utc: UtcMillis(200),
            monotonic: MonoInstant(1),
        },
    )
    .unwrap();
    tx.commit().unwrap();
}
