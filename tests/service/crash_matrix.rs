//! ht-4is.11.1: real-store composed-service failure matrix.
//!
//! Every test drives the production `DomainService` over a real SQLite file
//! opened through the production `StoreContext`, with an injected manual
//! clock, a fake host/notifier and test-only failpoints
//! (`crate::test_support::failpoints`). Assertions separate three facts:
//! the caller-observed outcome, authoritative SQLite rows, and pending work.
//! Each test documents the mutation it kills.
use crate::{
    notification::policy::RetryConfig,
    ports::{
        ClosureEvidence, HostCallContext, LocalService, NotificationPort, StorePort, WakeOutcome,
        WakeReservation,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{
            Accept, Ack, CheckIn, CheckInMode, Command, CreateThread, Invite, Leave,
            PendingReceiptsQuery, SendMessage, ThreadMutation,
        },
        ids::*,
        pagination::PageRequest,
        results::{AckResult, ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    scheduler::Scheduler,
    service::{dispatch::DomainService, fair_writer::FairWriter, workers::ScheduledStore},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::failpoints::Failpoint,
};
use rusqlite::{Connection, params};
use std::{
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
    time::Duration,
};

const OWNER: u32 = 501;
const RECEIPT_MS: i64 = 300_000;
const WAIT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------- fixture

struct ManualClock {
    utc: AtomicI64,
    mono: AtomicU64,
}
impl ManualClock {
    fn utc(&self) -> i64 {
        self.utc.load(Ordering::SeqCst)
    }
}
impl Clock for ManualClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}

/// Owns the private scratch directory; removed last.
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Matrix {
    clock: Arc<ManualClock>,
    instance: String,
    db_path: PathBuf,
    store: Arc<SqliteStore>,
    /// The production lane seam over the same store, as the workers use it.
    ports: ScheduledStore,
    service: DomainService,
    boot: uuid::Uuid,
    executions: AtomicU64,
    // Declared last so the SQLite handles above drop before the directory.
    _scratch: Scratch,
}

impl Matrix {
    fn new() -> Self {
        Self::with_instance(format!(
            "crash-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        ))
    }

    /// The shipped CLI names instances by UUID (its private context journal
    /// and claims carry `uuid::Uuid`), so CLI-driven rows use one.
    fn with_uuid_instance() -> Self {
        Self::with_instance(uuid::Uuid::new_v4().to_string())
    }

    fn with_instance(instance: String) -> Self {
        let root = std::env::temp_dir().join(format!("ht-crash-matrix-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let clock = Arc::new(ManualClock {
            utc: AtomicI64::new(1_000_000),
            mono: AtomicU64::new(1_000),
        });
        let db_path = root.join("store.db");
        let context = StoreContext::new(db_path.clone(), clock.clone());
        let db = context.open_writer().unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES (?1,0,'b',1)",
            [&instance],
        )
        .unwrap();
        drop(db);
        let boot = uuid::Uuid::new_v4();
        let (store, service) = Self::compose(&clock, &instance, &db_path, boot);
        Self {
            clock,
            instance,
            db_path,
            ports: ScheduledStore::new(store.clone(), Arc::new(FairWriter::new(32))),
            store,
            service,
            boot,
            executions: AtomicU64::new(1),
            _scratch: Scratch(root),
        }
    }

    fn compose(
        clock: &Arc<ManualClock>,
        instance: &str,
        db_path: &Path,
        boot: uuid::Uuid,
    ) -> (Arc<SqliteStore>, DomainService) {
        let clock: Arc<dyn Clock> = clock.clone();
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(db_path.to_path_buf(), clock.clone()),
                instance,
                StoreSettings {
                    daemon_boot: Some(boot),
                    // These crash boundaries exercise an immediate first wake;
                    // retained retry spacing still uses its production default.
                    wake_batch_delay_ms: 0,
                    ..StoreSettings::default()
                },
            )
            .unwrap(),
        );
        let port: Arc<dyn StorePort> = store.clone();
        let service = DomainService::new(instance.into(), port, clock)
            .with_operator_owner(OWNER)
            .with_cooperative_owner(OWNER, Arc::new(FairWriter::new(32)));
        (store, service)
    }

    /// Model a daemon crash/restart: every in-memory object is dropped and a
    /// new store, service and daemon boot are composed over the same file.
    fn restart(&mut self) {
        self.boot = uuid::Uuid::new_v4();
        let (store, service) = Self::compose(&self.clock, &self.instance, &self.db_path, self.boot);
        self.service = service;
        self.ports = ScheduledStore::new(store.clone(), Arc::new(FairWriter::new(32)));
        self.store = store;
    }

    fn scope(&self) -> String {
        self.db_path.display().to_string()
    }

    fn db(&self) -> Connection {
        Connection::open(&self.db_path).unwrap()
    }

    fn count(&self, sql: &str, args: impl rusqlite::Params) -> i64 {
        self.db().query_row(sql, args, |r| r.get(0)).unwrap()
    }

    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + 10_000),
            cancellation: Cancellation::default(),
        }
    }

    fn add_seat(&self, seat: &str) {
        let db = self.db();
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,?2,'resolved','native',?1,0,0,0)", params![seat, self.instance]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,?2,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?2,'inc','coherent_enumeration',1)", params![self.instance, seat]).unwrap();
    }

    fn handle(&self, command: Command) -> Result<CommandResult, ApiError> {
        self.service
            .handle(command, PeerIdentity::from_kernel(OWNER), &self.budget())
    }

    /// Lifecycle check-in by a fresh execution; returns the verified context.
    fn check_in(&self, seat: &str) -> CallerClaim {
        let n = self.executions.fetch_add(1, Ordering::SeqCst);
        let claim = CallerClaim {
            instance: self.instance.clone(),
            seat: SeatId::new(seat),
            binding_generation: 0,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new(format!("plugin_context:{seat}")),
            execution: ExecutionId::new(format!("00000000-0000-4000-8000-{n:012}")),
            target: HostTargetId::new(seat),
        };
        self.lifecycle(claim, &format!("checkin-{seat}-{n}"))
    }

    /// The same execution checks in again after a daemon restart.
    fn check_in_again(&self, claim: &CallerClaim) -> CallerClaim {
        let n = self.executions.fetch_add(1, Ordering::SeqCst);
        self.handle(Command::CheckIn(CheckIn {
            mode: CheckInMode::Current,
            claim: claim.clone(),
            operation: OperationId::new(format!("current-{}-{n}", claim.seat.as_str())),
        }))
        .map(|result| match result {
            CommandResult::CheckedIn(result) => result.context,
            other => panic!("unexpected {other:?}"),
        })
        .unwrap()
    }

    fn lifecycle(&self, mut claim: CallerClaim, operation: &str) -> CallerClaim {
        let current: i64 = self
            .db()
            .query_row(
                "SELECT generation FROM seats WHERE id=?1",
                [claim.seat.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        claim.binding_generation = current as u64;
        match self
            .handle(Command::CheckIn(CheckIn {
                mode: CheckInMode::Lifecycle {
                    expected_binding_generation: claim.binding_generation,
                },
                claim,
                operation: OperationId::new(operation),
            }))
            .unwrap()
        {
            CommandResult::CheckedIn(result) => result.context,
            other => panic!("unexpected {other:?}"),
        }
    }

    fn create_thread(&self, claim: &CallerClaim, op: &str) -> ThreadId {
        match self
            .handle(Command::CreateThread(CreateThread {
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new(op),
                claim: claim.clone(),
            }))
            .unwrap()
        {
            CommandResult::ThreadCreated(thread) => thread,
            other => panic!("unexpected {other:?}"),
        }
    }

    fn invite(&self, claim: &CallerClaim, thread: &ThreadId, seat: &str, op: &str) {
        self.invite_with_deadline(claim, thread, seat, None, op);
    }

    fn invite_with_deadline(
        &self,
        claim: &CallerClaim,
        thread: &ThreadId,
        seat: &str,
        deadline_millis: Option<u64>,
        op: &str,
    ) {
        self.handle(Command::Invite(Invite {
            thread: thread.clone(),
            seat: SeatId::new(seat),
            deadline_millis,
            operation: OperationId::new(op),
            claim: claim.clone(),
        }))
        .unwrap();
    }

    fn accept(
        &self,
        claim: &CallerClaim,
        thread: &ThreadId,
        op: &str,
    ) -> Result<CommandResult, ApiError> {
        self.handle(Command::Accept(Accept {
            thread: thread.clone(),
            operation: OperationId::new(op),
            claim: claim.clone(),
        }))
    }

    fn join(&self, owner: &CallerClaim, thread: &ThreadId, member: &CallerClaim) {
        let seat = member.seat.as_str();
        self.invite(
            owner,
            thread,
            seat,
            &format!("invite-{seat}-{}", thread.as_str()),
        );
        self.accept(
            member,
            thread,
            &format!("accept-{seat}-{}", thread.as_str()),
        )
        .unwrap();
    }

    fn send(
        &self,
        claim: &CallerClaim,
        thread: &ThreadId,
        body: &str,
        invited: &[&str],
        op: &str,
    ) -> Result<MessageId, ApiError> {
        self.handle(Command::SendMessage(SendMessage {
            thread: thread.clone(),
            body: body.into(),
            invited_recipients: invited.iter().map(|s| SeatId::new(*s)).collect(),
            deadline_millis: None,
            operation: OperationId::new(op),
            claim: claim.clone(),
            relays_user: false,
        }))
        .map(|result| match result {
            CommandResult::MessageSent(id) => id,
            other => panic!("unexpected {other:?}"),
        })
    }

    fn ack(
        &self,
        claim: &CallerClaim,
        messages: &[&MessageId],
        op: &str,
    ) -> Result<AckResult, ApiError> {
        self.handle(Command::Ack(Ack {
            messages: messages.iter().map(|m| (*m).clone()).collect(),
            operation: OperationId::new(op),
            claim: claim.clone(),
        }))
        .map(|result| match result {
            CommandResult::Acknowledged(ack) => ack,
            other => panic!("unexpected {other:?}"),
        })
    }

    /// Client-observable pending receipts, traversing every returned page.
    fn pending(&self, seat: &str) -> Vec<MessageId> {
        let mut page = PageRequest::default();
        let mut out = Vec::new();
        loop {
            let CommandResult::PendingReceipts(result) = self
                .handle(Command::PendingReceipts(PendingReceiptsQuery {
                    seat: Some(SeatId::new(seat)),
                    thread: None,
                    page: page.clone(),
                }))
                .unwrap()
            else {
                panic!("wrong result")
            };
            out.extend(result.items.into_iter().map(|item| item.message));
            if !result.has_more {
                return out;
            }
            page.cursor = Some(result.next_cursor.expect("continuation"));
        }
    }

    fn message_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM messages WHERE kind='ordinary'", [])
    }

    fn overdue_events(&self, message: &MessageId, seat: &str) -> i64 {
        self.count(
            "SELECT count(*) FROM messages WHERE kind='warn' AND event_key=?1",
            [format!(
                "overdue:receipt:{}:{}:{}",
                message.as_str().len(),
                message.as_str(),
                seat
            )],
        )
    }

    fn ack_events(&self, seat: &str) -> i64 {
        self.count(
            "SELECT count(*) FROM messages WHERE kind='info' AND event_key LIKE ?1",
            [format!("ack:{}:{}:%", seat.len(), seat)],
        )
    }

    fn receipt_state(&self, message: &MessageId, seat: &str) -> String {
        let db = self.db();
        let physical: Option<String> = db
            .query_row(
                "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
                params![message.as_str(), seat],
                |r| r.get(0),
            )
            .ok();
        physical.unwrap_or_else(|| "pending".into())
    }

    fn recipient_snapshot(&self, message: &MessageId) -> Vec<String> {
        let db = self.db();
        let mut stmt = db
            .prepare("SELECT r.seat_id FROM prepared_recipients r JOIN send_manifests m ON m.preparation_id=r.preparation_id WHERE m.message_id=?1 ORDER BY r.seat_id")
            .unwrap();
        stmt.query_map([message.as_str()], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// Drive the production deadline/work lane (materialization, due scans,
    /// cleanup) until it reports no more work, using the real scheduler.
    fn drive_deadlines(&self) -> u32 {
        let notifier = PromptLog::default();
        let scheduler = Scheduler::new(
            self.instance.clone(),
            &self.ports,
            &self.ports,
            &notifier,
            RetryConfig::default(),
            self.boot,
        );
        let mut warnings = 0u32;
        for _ in 0..64 {
            scheduler.after_committed_fence().unwrap();
            let outcome = scheduler.drive_deadlines(&self.budget()).unwrap();
            assert_eq!(outcome.work_error, None);
            assert_eq!(outcome.retirement_error, None);
            warnings += u32::from(outcome.due_warnings_added);
            if !outcome.due_continuation
                && outcome.work_job.is_none()
                && outcome.retirement_job.is_none()
            {
                return warnings;
            }
        }
        panic!("deadline lane did not settle");
    }

    /// Two members in one thread: sender `s` and recipient `r`, both checked
    /// in (so the receipt timer starts at send).
    fn pair(&self) -> (CallerClaim, CallerClaim, ThreadId) {
        self.add_seat("s");
        self.add_seat("r");
        let s = self.check_in("s");
        let r = self.check_in("r");
        let thread = self.create_thread(&s, "thread");
        self.join(&s, &thread, &r);
        (s, r, thread)
    }
}

/// Fake host prompt boundary: records each submitted prompt.
#[derive(Default)]
struct PromptLog {
    prompts: Mutex<Vec<(SeatId, WakeAttemptId)>>,
}
impl NotificationPort for PromptLog {
    fn attempt_wake(
        &self,
        reservation: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.prompts
            .lock()
            .unwrap()
            .push((reservation.seat.clone(), reservation.attempt.clone()));
        Ok(WakeOutcome::Submitted)
    }
}

// --------------------------------------------------------------- matrix

/// Boundary: before send commit.
/// Kills: a send that becomes durable (operation, message, manifest, work or
/// wake) although its decision transaction failed before COMMIT, and a retry
/// that duplicates or cannot commit.
#[test]
fn send_failure_before_commit_leaves_no_acceptance_and_retry_commits_once() {
    let m = Matrix::new();
    let (s, _r, thread) = m.pair();
    let messages_before = m.message_rows();
    let ops_before = m.count("SELECT count(*) FROM operations", []);
    {
        let _fp = Failpoint::error("send.before_commit", m.scope(), ErrorCode::StoreBusy);
        let error = m.send(&s, &thread, "hello", &[], "send-1").unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreBusy);
        assert_eq!(_fp.fired(), 1);
    }
    // Authoritative facts: nothing accepted.
    assert_eq!(m.message_rows(), messages_before);
    assert_eq!(m.count("SELECT count(*) FROM operations", []), ops_before);
    assert_eq!(m.count("SELECT count(*) FROM send_manifests", []), 0);
    assert_eq!(m.count("SELECT count(*) FROM receipt_state", []), 0);
    assert_eq!(
        m.count(
            "SELECT count(*) FROM work_jobs WHERE kind='send_attention'",
            []
        ),
        0
    );
    // Pending work: the recipient has nothing to read, and no wake is due.
    assert!(m.pending("r").is_empty());
    let wake =
        StorePort::wake_candidates(m.store.as_ref(), PageRequest::default(), &m.budget()).unwrap();
    assert!(
        wake.items.iter().all(|c| !c.has_pending_receipt),
        "failed commit must never wake: {wake:?}"
    );
    // Retry with the same key commits exactly once.
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    assert_eq!(m.message_rows(), messages_before + 1);
    assert_eq!(m.send(&s, &thread, "hello", &[], "send-1").unwrap(), sent);
    assert_eq!(m.message_rows(), messages_before + 1);
    assert_eq!(m.pending("r"), vec![sent]);
}

/// Boundary: after send commit, before response.
/// Kills: a replay that re-decides (new message ID, new recipient snapshot,
/// duplicate receipts) instead of returning the original durable result.
#[test]
fn send_response_lost_after_commit_recovers_original_message_and_snapshot() {
    let mut m = Matrix::new();
    let (s, _r, thread) = m.pair();
    m.add_seat("x");
    let x = m.check_in("x");
    m.join(&s, &thread, &x);
    {
        let _fp = Failpoint::error("send.after_commit", m.scope(), ErrorCode::UnknownOutcome);
        let error = m.send(&s, &thread, "hello", &[], "send-1").unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownOutcome);
    }
    // The caller saw a failure, yet SQLite committed exactly one message.
    assert_eq!(m.message_rows(), 1);
    let committed: String = m
        .db()
        .query_row("SELECT id FROM messages WHERE kind='ordinary'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let snapshot = m.recipient_snapshot(&MessageId::new(&committed));
    assert_eq!(snapshot, vec!["r".to_string(), "x".to_string()]);
    // A membership change after the commit must not leak into the replay.
    m.handle(Command::Leave(Leave {
        thread: thread.clone(),
        operation: OperationId::new("x-leaves"),
        claim: x.clone(),
    }))
    .unwrap();
    // The daemon also restarts before the caller retries.
    m.restart();
    let recovered = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    assert_eq!(recovered.as_str(), committed);
    assert_eq!(m.message_rows(), 1);
    assert_eq!(m.recipient_snapshot(&recovered), snapshot);
    assert_eq!(m.count("SELECT count(*) FROM send_manifests", []), 1);
}

/// Boundary: ACK commit, before response.
/// Kills: an ACK replay that re-decides (second info event, changed actor or
/// result) instead of returning the original immutable result.
#[test]
fn ack_response_lost_after_commit_replays_original_actor_and_one_event() {
    let mut m = Matrix::new();
    let (s, r, thread) = m.pair();
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    {
        let _fp = Failpoint::error("ack.after_commit", m.scope(), ErrorCode::UnknownOutcome);
        let error = m.ack(&r, &[&sent], "ack-1").unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownOutcome);
    }
    assert_eq!(m.receipt_state(&sent, "r"), "acked");
    assert_eq!(m.ack_events("r"), 1);
    let actor: (String, i64) = m
        .db()
        .query_row(
            "SELECT ack_actor_seat_id,ack_generation FROM receipt_state WHERE message_id=?1",
            [sent.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    m.restart();
    let replay = m.ack(&r, &[&sent], "ack-1").unwrap();
    assert_eq!(
        replay,
        AckResult {
            acknowledged: vec![sent.clone()],
            already_acknowledged: vec![],
        }
    );
    assert_eq!(m.ack_events("r"), 1);
    let after: (String, i64) = m
        .db()
        .query_row(
            "SELECT ack_actor_seat_id,ack_generation FROM receipt_state WHERE message_id=?1",
            [sent.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after, actor);
    assert!(m.pending("r").is_empty());
}

/// Boundary: ACK failure before COMMIT (writes done, commit not reached).
/// Kills: a partially applied ACK (receipt settled or info event emitted)
/// surviving a failed decision, i.e. a false ACK.
#[test]
fn ack_failure_before_commit_is_not_a_receipt() {
    let m = Matrix::new();
    let (s, r, thread) = m.pair();
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    {
        let _fp = Failpoint::error("ack.before_commit", m.scope(), ErrorCode::StoreBusy);
        assert_eq!(
            m.ack(&r, &[&sent], "ack-1").unwrap_err().code,
            ErrorCode::StoreBusy
        );
    }
    assert_eq!(m.receipt_state(&sent, "r"), "pending");
    assert_eq!(m.ack_events("r"), 0);
    assert_eq!(m.pending("r"), vec![sent.clone()]);
    assert_eq!(
        m.ack(&r, &[&sent], "ack-1").unwrap().acknowledged,
        vec![sent]
    );
    assert_eq!(m.ack_events("r"), 1);
}

impl Matrix {
    fn scheduler<'a>(
        &'a self,
        notifier: &'a PromptLog,
    ) -> Scheduler<'a, ScheduledStore, ScheduledStore, PromptLog> {
        Scheduler::new(
            self.instance.clone(),
            &self.ports,
            &self.ports,
            notifier,
            RetryConfig::default(),
            self.boot,
        )
    }

    fn wake_row(&self, seat: &str) -> (Option<String>, Option<String>, Option<String>) {
        self.db()
            .query_row(
                "SELECT reservation_id,reservation_boot,last_outcome FROM wake_work WHERE seat_id=?1",
                [seat],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    /// The fake host's fresh current-target observation: an idle top-level
    /// occupant running the checked-in execution (the only wakeable state).
    fn host_observes_idle(&self, claim: &CallerClaim) {
        self.db()
            .execute(
                "UPDATE observed_targets SET occupancy='occupied',ui_state='idle',top_level_occupant=1,verified_execution=?1 WHERE instance_id=?2 AND target_id=?3",
                params![claim.execution.as_str(), self.instance, claim.target.as_str()],
            )
            .unwrap();
    }

    fn set_mono(&self, mono: u64) {
        self.clock.mono.store(mono, Ordering::SeqCst);
    }
}

/// Production defaults delay the first ordinary wake without sliding the
/// retained deadline on new mail or making batch bookkeeping self-kick.
#[test]
fn default_batching_retains_first_attention_without_self_kicks() {
    use crate::service::kicks::{CommitKicks, Lane, LaneSet, enter_lane};

    let m = Matrix::new();
    let (s, r, thread) = m.pair();
    m.host_observes_idle(&r);
    let sent = m.send(&s, &thread, "first", &[], "batch-first").unwrap();
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(m.db_path.clone(), m.clock.clone()),
            m.instance.clone(),
            StoreSettings {
                daemon_boot: Some(m.boot),
                ..StoreSettings::default()
            },
        )
        .unwrap()
        .with_commit_kicks(Arc::new(CommitKicks::default())),
    );
    let kicks = Arc::new(Mutex::new(Vec::<LaneSet>::new()));
    let captured = kicks.clone();
    store.set_kick_sink(Box::new(move |lanes, _| {
        captured.lock().unwrap().push(lanes)
    }));
    let ports = ScheduledStore::new(store, Arc::new(FairWriter::new(32)));
    let notifier = PromptLog::default();
    let scheduler = Scheduler::new(
        m.instance.clone(),
        &ports,
        &ports,
        &notifier,
        RetryConfig::default(),
        m.boot,
    );
    let _origin = enter_lane(Lane::Wakes);
    let first = scheduler.drive_wakes(&m.budget()).unwrap();
    assert_eq!(first.attempted, 0);
    assert_eq!(first.next_due_at, Some(MonoInstant(31_000)));
    assert_eq!(m.wake_row("r"), (None, None, None));
    assert!(
        kicks.lock().unwrap().is_empty(),
        "batch retention must not self-kick"
    );

    // A later arrival must not slide the original durable deadline.
    m.clock.utc.store(1_020_000, Ordering::SeqCst);
    m.set_mono(21_000);
    m.send(&s, &thread, "later", &[], "batch-later").unwrap();
    assert_eq!(
        scheduler.drive_wakes(&m.budget()).unwrap().next_due_at,
        Some(MonoInstant(31_000))
    );
    m.clock.utc.store(1_029_999, Ordering::SeqCst);
    m.set_mono(30_999);
    assert_eq!(scheduler.drive_wakes(&m.budget()).unwrap().attempted, 0);
    m.clock.utc.store(1_030_000, Ordering::SeqCst);
    m.set_mono(31_000);
    assert_eq!(scheduler.drive_wakes(&m.budget()).unwrap().attempted, 1);
    assert_eq!(notifier.prompts.lock().unwrap().len(), 1);
    assert_eq!(m.receipt_state(&sent, "r"), "pending");
}

/// Boundary: before wake. The send committed, then the daemon went down
/// before any wake reservation existed.
/// Kills: restart recovery that rebuilds wake work only from in-memory send
/// notifications (or treats never-reserved work as previously reserved and
/// holds it for a retained delay) instead of from durable rows.
#[test]
fn crash_after_send_before_any_wake_reservation_wakes_from_durable_rows() {
    let mut m = Matrix::new();
    let (s, r, thread) = m.pair();
    m.host_observes_idle(&r);
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    // Nothing was reserved before the crash.
    assert_eq!(m.wake_row("r"), (None, None, None));
    m.restart();
    let notifier = PromptLog::default();
    let scheduler = m.scheduler(&notifier);
    let outcome = scheduler.drive_wakes(&m.budget()).unwrap();
    assert_eq!(
        (outcome.recovered, outcome.examined, outcome.attempted),
        (0, 1, 1),
        "{outcome:?}"
    );
    let prompts = notifier.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].0.as_str(), "r");
    let (reservation, _, outcome) = m.wake_row("r");
    assert_eq!((reservation, outcome.as_deref()), (None, Some("submitted")));
    let (last_reservation, last_boot): (Option<String>, Option<String>) = m
        .db()
        .query_row(
            "SELECT last_reservation_id,last_reservation_boot FROM wake_work WHERE seat_id='r'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        last_reservation.as_deref(),
        Some(prompts[0].1.as_str()),
        "the prompt used the committed reservation"
    );
    assert_eq!(
        last_boot,
        Some(m.boot.to_string()),
        "reserved by the new daemon"
    );
    // A wake is never a receipt.
    assert_eq!(m.receipt_state(&sent, "r"), "pending");
    assert_eq!(m.pending("r"), vec![sent]);
    assert_eq!(m.ack_events("r"), 0);
}

/// Boundary: after wake reservation commit, before the prompt.
/// Kills: restart recovery that forgets the persisted reservation (lost work)
/// or resets retry spacing so the new daemon prompts immediately.
#[test]
fn crash_after_wake_reservation_recovers_work_and_keeps_persisted_spacing() {
    let mut m = Matrix::new();
    let (s, r, thread) = m.pair();
    m.host_observes_idle(&r);
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    let crashed_boot = m.boot;
    {
        let notifier = PromptLog::default();
        let scheduler = m.scheduler(&notifier);
        let _fp = Failpoint::error(
            "wake.after_reservation",
            crashed_boot.to_string(),
            ErrorCode::Cancelled,
        );
        assert!(scheduler.drive_wakes(&m.budget()).is_err());
        assert_eq!(_fp.fired(), 1);
        assert!(
            notifier.prompts.lock().unwrap().is_empty(),
            "no prompt before crash"
        );
    }
    let (reservation, boot, outcome) = m.wake_row("r");
    assert!(reservation.is_some(), "reservation committed before crash");
    assert_eq!(boot, Some(crashed_boot.to_string()));
    assert_eq!(outcome, None);

    m.restart();
    let restarted_at = m.clock.monotonic_now().0;
    let notifier = PromptLog::default();
    let scheduler = m.scheduler(&notifier);
    let first = scheduler.drive_wakes(&m.budget()).unwrap();
    assert_eq!((first.recovered, first.attempted), (1, 0));
    assert_eq!(m.wake_row("r").2.as_deref(), Some("outcome_unknown"));
    // Persisted minimum spacing (30 s default) holds across the restart.
    m.set_mono(restarted_at + 29_999);
    assert_eq!(scheduler.drive_wakes(&m.budget()).unwrap().attempted, 0);
    assert!(notifier.prompts.lock().unwrap().is_empty());
    m.set_mono(restarted_at + 30_000);
    assert_eq!(scheduler.drive_wakes(&m.budget()).unwrap().attempted, 1);
    let prompts = notifier.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].0.as_str(), "r");
    // Work reconstructed: the receipt is still pending, never inferred.
    assert_eq!(m.pending("r"), vec![sent]);
}

/// Boundary: after the prompt, before its attempt record.
/// Kills: recovery that records the uncertain attempt as submitted, infers a
/// receipt from a prompt, or duplicates mail on the repeated hint.
#[test]
fn crash_after_prompt_before_attempt_record_is_honest_and_never_acks() {
    let mut m = Matrix::new();
    let (s, r, thread) = m.pair();
    m.host_observes_idle(&r);
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    let crashed_boot = m.boot;
    let first_prompts = {
        let notifier = PromptLog::default();
        let scheduler = m.scheduler(&notifier);
        let _fp = Failpoint::error(
            "wake.after_prompt",
            crashed_boot.to_string(),
            ErrorCode::Cancelled,
        );
        assert!(scheduler.drive_wakes(&m.budget()).is_err());
        assert_eq!(_fp.fired(), 1);
        notifier.prompts.lock().unwrap().len()
    };
    assert_eq!(
        first_prompts, 1,
        "the prompt was submitted before the crash"
    );
    let (reservation, _, outcome) = m.wake_row("r");
    assert!(
        reservation.is_some() && outcome.is_none(),
        "no attempt record"
    );

    m.restart();
    let restarted_at = m.clock.monotonic_now().0;
    let notifier = PromptLog::default();
    let scheduler = m.scheduler(&notifier);
    assert_eq!(scheduler.drive_wakes(&m.budget()).unwrap().recovered, 1);
    // Honest record: the recovered attempt is unknown, not submitted.
    assert_eq!(m.wake_row("r").2.as_deref(), Some("outcome_unknown"));
    m.set_mono(restarted_at + 30_000);
    assert_eq!(scheduler.drive_wakes(&m.budget()).unwrap().attempted, 1);
    // A repeated hint is possible and counted separately from mail.
    assert_eq!(notifier.prompts.lock().unwrap().len(), 1);
    assert_eq!(m.message_rows(), 1, "no duplicate mail");
    assert_eq!(m.receipt_state(&sent, "r"), "pending", "no inferred ACK");
    assert_eq!(m.ack_events("r"), 0);
    assert_eq!(m.pending("r"), vec![sent.clone()]);
    // Only the explicit ACK settles it.
    assert_eq!(
        m.ack(&r, &[&sent], "ack-1").unwrap().acknowledged,
        vec![sent]
    );
}

impl Matrix {
    fn deadline(&self, message: &MessageId, seat: &str) -> i64 {
        let CommandResult::PendingReceipts(page) = self
            .handle(Command::PendingReceipts(PendingReceiptsQuery {
                seat: Some(SeatId::new(seat)),
                thread: None,
                page: PageRequest::default(),
            }))
            .unwrap()
        else {
            panic!("wrong result")
        };
        page.items
            .iter()
            .find(|item| &item.message == message)
            .and_then(|item| item.deadline)
            .expect("started receipt deadline")
            .0
    }

    fn set_utc(&self, utc: i64) {
        self.clock.utc.store(utc, Ordering::SeqCst);
    }

    /// Timeline sequence of the single event with `key`.
    fn event_sequence(&self, key_like: &str) -> Vec<i64> {
        let db = self.db();
        let mut stmt = db
            .prepare("SELECT sequence FROM messages WHERE event_key LIKE ?1 ORDER BY sequence")
            .unwrap();
        stmt.query_map([key_like], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

/// Boundary: ACK decided at (equality) or after the deadline while the daemon
/// was down (no due scan ran).
/// Kills: late settlement without its missed-deadline event, a warning after
/// the settlement, or a second warning from the next due scan.
#[test]
fn late_ack_after_downtime_records_one_overdue_event_before_settlement() {
    for (label, late_by) in [("equality", 0), ("after", 45_000)] {
        let mut m = Matrix::new();
        let (s, r, thread) = m.pair();
        let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
        let deadline = m.deadline(&sent, "r");
        assert_eq!(deadline, m.clock.utc() + RECEIPT_MS, "{label}");
        // Daemon down across the deadline: nothing scans.
        m.set_utc(deadline + late_by);
        m.clock.mono.fetch_add(RECEIPT_MS as u64, Ordering::SeqCst);
        m.restart();
        let r = m.check_in_again(&r);
        assert_eq!(
            m.ack(&r, &[&sent], "ack-late").unwrap().acknowledged,
            vec![sent.clone()]
        );
        assert_eq!(m.overdue_events(&sent, "r"), 1, "{label}");
        let warning = m.event_sequence("overdue:receipt:%");
        let settled = m.event_sequence("ack:%");
        assert_eq!((warning.len(), settled.len()), (1, 1), "{label}");
        assert!(
            warning[0] < settled[0],
            "{label}: warning must precede settlement"
        );
        assert_eq!(m.receipt_state(&sent, "r"), "acked");
        // The next due scan cannot add a second event.
        assert_eq!(m.drive_deadlines(), 0, "{label}");
        assert_eq!(m.overdue_events(&sent, "r"), 1, "{label}");
    }
}

/// Boundary: invitation accept decided after its deadline during downtime.
/// Kills: late accept without its single missed-deadline event, or ordering
/// the event after the acceptance.
#[test]
fn late_accept_after_downtime_records_one_overdue_event_before_acceptance() {
    let mut m = Matrix::new();
    m.add_seat("s");
    m.add_seat("x");
    let s = m.check_in("s");
    let x = m.check_in("x");
    let thread = m.create_thread(&s, "thread");
    m.invite_with_deadline(&s, &thread, "x", Some(60_000), "invite-x");
    m.set_utc(m.clock.utc() + 60_000);
    m.clock.mono.fetch_add(60_000, Ordering::SeqCst);
    m.restart();
    let x = m.check_in_again(&x);
    m.accept(&x, &thread, "accept-x").unwrap();
    let warning = m.event_sequence("overdue:invitation:%");
    assert_eq!(warning.len(), 1);
    let accepted: i64 = m.count(
        "SELECT max(sequence) FROM messages WHERE thread_id=?1 AND kind='info'",
        [thread.as_str()],
    );
    assert!(warning[0] < accepted, "warning must precede acceptance");
    assert_eq!(m.drive_deadlines(), 0);
    assert_eq!(m.event_sequence("overdue:invitation:%").len(), 1);
}

/// Race: a timely ACK decided before the deadline but paused (holding the
/// writer) until after it, while a due scan is proven (by a writer-contention
/// barrier, not a sleep) to be waiting for that writer.
/// Kills: ACK reclassification by physical commit time. (`scan_due` reads its
/// candidates inside its own writer transaction, so a stale-candidate-read
/// mutation is not expressible here and is not claimed.)
#[test]
fn timely_ack_paused_across_deadline_with_waiting_due_scan_never_warns() {
    let m = Matrix::new();
    let (s, r, thread) = m.pair();
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    let deadline = m.deadline(&sent, "r");
    m.set_utc(deadline - 1);
    let pause = Failpoint::pause("ack.after_decision_sample", m.scope());
    std::thread::scope(|scope| {
        let acking = scope.spawn(|| m.ack(&r, &[&sent], "ack-1"));
        assert!(pause.wait_fired(1, WAIT), "ACK reached its decision sample");
        // Decision sampled at deadline-1; UTC now crosses the deadline.
        m.set_utc(deadline + 10);
        m.clock.mono.fetch_add(11, Ordering::SeqCst);
        let contended = Failpoint::observe("store.writer.contended", m.scope());
        let scanning = scope.spawn(|| m.drive_deadlines());
        let reached_writer = contended.wait_fired(1, WAIT);
        let waiting = !scanning.is_finished();
        // Release before asserting so a failure fails instead of hanging the
        // scope on the paused ACK thread.
        pause.release();
        assert!(
            reached_writer,
            "due scan reached the writer while the ACK holds it"
        );
        assert!(waiting, "due scan waits for the writer");
        assert_eq!(
            acking.join().unwrap().unwrap().acknowledged,
            vec![sent.clone()]
        );
        assert_eq!(scanning.join().unwrap(), 0);
    });
    assert_eq!(m.overdue_events(&sent, "r"), 0);
    assert_eq!(m.receipt_state(&sent, "r"), "acked");
    assert_eq!(m.drive_deadlines(), 0);
    assert_eq!(m.overdue_events(&sent, "r"), 0);
}

/// Race: join and leave land between two bounded send-preparation quanta.
/// Kills: publishing a stale recipient snapshot (includes the leaver, misses
/// the joiner), partial publication, and duplicate staging of an explicit
/// invited target that is also reached through membership.
#[test]
fn join_and_leave_between_send_quanta_yield_one_exact_recipient_snapshot() {
    let m = Matrix::new();
    m.add_seat("s");
    let s = m.check_in("s");
    let thread = m.create_thread(&s, "thread");
    let mut members = Vec::new();
    for n in 0..16 {
        let seat = format!("r{n:02}");
        m.add_seat(&seat);
        let claim = m.check_in(&seat);
        m.join(&s, &thread, &claim);
        members.push(claim);
    }
    for seat in ["x", "late"] {
        m.add_seat(seat);
    }
    let late = m.check_in("late");
    m.invite(&s, &thread, "x", "invite-x");
    m.invite(&s, &thread, "late", "invite-late");
    let leaver = members.pop().unwrap();
    // 17 membership units plus 2 explicit targets exceed one 16-unit quantum.
    let explicit = ["x", "r00"];
    let pause = Failpoint::pause("send.between_preparation_steps", m.instance.clone());
    let first = std::thread::scope(|scope| {
        let sending = scope.spawn(|| m.send(&s, &thread, "hello", &explicit, "send-1"));
        assert!(pause.wait_fired(1, WAIT), "send yielded between quanta");
        m.accept(&late, &thread, "accept-late").unwrap();
        m.handle(Command::Leave(Leave {
            thread: thread.clone(),
            operation: OperationId::new("leave"),
            claim: leaver.clone(),
        }))
        .unwrap();
        pause.release();
        sending.join().unwrap()
    });
    drop(pause);
    // The stale preparation is never published.
    assert_eq!(first.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(m.message_rows(), 0);
    assert_eq!(m.count("SELECT count(*) FROM send_manifests", []), 0);
    // Bounded cleanup runs, then the exact-key retry prepares afresh.
    m.drive_deadlines();
    let sent = m.send(&s, &thread, "hello", &explicit, "send-1").unwrap();
    let mut expected: Vec<String> = members
        .iter()
        .map(|c| c.seat.as_str().to_string())
        .collect();
    expected.extend(["late".to_string(), "x".to_string()]);
    expected.sort();
    assert_eq!(m.recipient_snapshot(&sent), expected);
    assert_eq!(m.message_rows(), 1);
    // The immutable snapshot survives later membership change.
    m.handle(Command::Leave(Leave {
        thread: thread.clone(),
        operation: OperationId::new("leave-late"),
        claim: late.clone(),
    }))
    .unwrap();
    assert_eq!(m.recipient_snapshot(&sent), expected);
    assert_eq!(m.pending("late"), vec![sent.clone()]);
    assert!(m.pending(leaver.seat.as_str()).is_empty());
}

/// Boundary: archive while a receipt is pending, deadline passes.
/// Kills: archive that cancels/hides pending obligations or rejects the
/// late settlement.
#[test]
fn archive_during_pending_work_allows_late_settlement() {
    let m = Matrix::new();
    let (s, r, thread) = m.pair();
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    let deadline = m.deadline(&sent, "r");
    m.handle(Command::Archive(ThreadMutation {
        thread: thread.clone(),
        operation: OperationId::new("archive"),
        claim: s.clone(),
    }))
    .unwrap();
    assert_eq!(
        m.pending("r"),
        vec![sent.clone()],
        "archived obligation stays discoverable"
    );
    m.set_utc(deadline + 1);
    m.clock.mono.fetch_add(RECEIPT_MS as u64, Ordering::SeqCst);
    assert_eq!(m.drive_deadlines(), 1);
    let r = m.check_in_again(&r);
    assert_eq!(
        m.ack(&r, &[&sent], "ack-late").unwrap().acknowledged,
        vec![sent.clone()]
    );
    assert_eq!(m.receipt_state(&sent, "r"), "acked");
    assert_eq!(m.overdue_events(&sent, "r"), 1);
    assert!(m.pending("r").is_empty());
}

/// Boundary: retirement fence while a receipt is pending.
/// Kills: a retired seat's receipt being ACKed (by its stale context or by
/// retirement cleanup) or reported as pending work.
#[test]
fn retirement_during_pending_work_never_acks() {
    let m = Matrix::new();
    let (s, r, thread) = m.pair();
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    StorePort::begin_retirement(
        m.store.as_ref(),
        SeatId::new("r"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("r"),
            generation: 0,
        },
        &m.budget(),
    )
    .unwrap();
    let error = m.ack(&r, &[&sent], "ack-after-retire").unwrap_err();
    // The retired seat's context is fenced; nothing can settle for it.
    assert_eq!(error.code, ErrorCode::CallerUnverified, "{error:?}");
    m.drive_deadlines();
    assert_ne!(m.receipt_state(&sent, "r"), "acked");
    assert_eq!(m.ack_events("r"), 0);
    assert!(
        m.pending("r").is_empty(),
        "retired obligations are not pending work"
    );
    assert_eq!(
        m.count(
            "SELECT count(*) FROM receipt_state WHERE state='acked' OR ack_actor_seat_id IS NOT NULL",
            []
        ),
        0
    );
    // A still-joined sender is unaffected.
    assert!(m.send(&s, &thread, "after", &[], "send-2").is_ok());
}

/// Boundary: rebind (successor execution) while a receipt is pending.
/// Kills: a predecessor context that can still settle, or a successor that
/// cannot; provenance must name the successor generation.
#[test]
fn rebind_during_pending_work_invalidates_old_context() {
    let m = Matrix::new();
    let (s, old, thread) = m.pair();
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    let successor = m.check_in("r");
    assert!(successor.binding_generation > old.binding_generation);
    let error = m.ack(&old, &[&sent], "ack-old").unwrap_err();
    assert_eq!(error.code, ErrorCode::CallerUnverified, "{error:?}");
    assert_eq!(m.receipt_state(&sent, "r"), "pending");
    assert_eq!(m.pending("r"), vec![sent.clone()]);
    m.ack(&successor, &[&sent], "ack-new").unwrap();
    let generation: i64 = m.count(
        "SELECT ack_generation FROM receipt_state WHERE message_id=?1",
        [sent.as_str()],
    );
    assert_eq!(generation as u64, successor.binding_generation);
}

/// Boundary: more than one inbox/history page, an output-budget failure and a
/// multi-byte body split across continuation reads.
/// Kills: dropped/duplicated items across page boundaries, a failed or
/// partial read treated as a receipt, and body splits inside a UTF-8 char.
#[test]
fn paged_reads_output_failure_and_partial_utf8_body_keep_every_obligation() {
    use crate::protocol::commands::{BodyReadRequest, HistoryQuery, MessageQuery};
    use crate::protocol::results::MessageContent;
    let m = Matrix::new();
    let (s, _r, thread) = m.pair();
    let body: String = "aé€😀".repeat(300); // 1+2+3+4 bytes per unit, 3000 bytes
    let mut sent = Vec::new();
    for n in 0..45 {
        let text = if n == 0 {
            body.clone()
        } else {
            format!("m{n}")
        };
        sent.push(
            m.send(&s, &thread, &text, &[], &format!("send-{n}"))
                .unwrap(),
        );
    }
    // Pending receipts: at least three pages, exact continuation, no loss.
    let mut page = PageRequest::default();
    let (mut seen, mut pages) = (Vec::new(), 0);
    loop {
        let CommandResult::PendingReceipts(result) = m
            .handle(Command::PendingReceipts(PendingReceiptsQuery {
                seat: Some(SeatId::new("r")),
                thread: None,
                page: page.clone(),
            }))
            .unwrap()
        else {
            panic!("wrong result")
        };
        pages += 1;
        seen.extend(result.items.into_iter().map(|item| item.message));
        if !result.has_more {
            break;
        }
        page.cursor = result.next_cursor;
    }
    assert!(pages >= 3, "{pages} pages");
    assert_eq!(seen, sent, "every obligation exactly once, in order");
    // History traversal reaches every ordinary message.
    let mut page = PageRequest::default();
    let mut history = Vec::new();
    loop {
        let CommandResult::History(result) = m
            .handle(Command::History(HistoryQuery {
                thread: thread.clone(),
                page: page.clone(),
                initial: None,
                full_bodies: false,
            }))
            .unwrap()
        else {
            panic!("wrong result")
        };
        history.extend(result.items.into_iter().map(|item| item.message));
        if !result.has_more {
            break;
        }
        page.cursor = result.next_cursor;
    }
    assert!(sent.iter().all(|id| history.contains(id)));
    // Output failure: a page budget too small to encode is a typed error
    // naming the minimum; nothing is settled by the failed read.
    let error = m
        .handle(Command::PendingReceipts(PendingReceiptsQuery {
            seat: Some(SeatId::new("r")),
            thread: None,
            page: PageRequest {
                cursor: None,
                limit: 20,
                max_bytes: 256,
            },
        }))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidBudget, "{error:?}");
    assert!(error.required_minimum_bytes.is_some_and(|min| min > 256));
    // Body continuation never splits a character and reassembles exactly.
    let read = |cursor: Option<String>, offset: Option<u64>| {
        m.handle(Command::Message(MessageQuery {
            message: sent[0].clone(),
            body: BodyReadRequest {
                cursor,
                offset,
                max_bytes: 2600,
            },
        }))
    };
    let (mut text, mut cursor, mut chunks) = (String::new(), None, 0);
    loop {
        let CommandResult::Message(detail) = read(cursor.clone(), None).unwrap() else {
            panic!("wrong result")
        };
        let MessageContent::Ordinary {
            body_data,
            body_offset,
            body_complete,
            body_next_cursor,
            ..
        } = detail.content
        else {
            panic!("ordinary body")
        };
        assert_eq!(body_offset as usize, text.len());
        assert!(!body_data.contains('\u{fffd}'));
        text.push_str(&body_data);
        chunks += 1;
        if body_complete {
            break;
        }
        cursor = body_next_cursor;
    }
    assert!(chunks >= 2, "{chunks} chunks");
    assert_eq!(text, body);
    // A raw offset inside a multi-byte character is rejected explicitly.
    assert_eq!(
        read(None, Some(2)).unwrap_err().code,
        ErrorCode::InvalidCursor
    );
    // Reads, failed reads and body continuations are never receipts.
    assert_eq!(m.pending("r"), sent);
    assert_eq!(m.ack_events("r"), 0);
}

/// Storage fault: SQLite reports SQLITE_FULL (real, via max_page_count on the
/// production writer connection) at the send decision.
/// Kills: a send accepted, partially written or operation-recorded while the
/// database cannot grow, history lost by the failed write, and disk full
/// reported as corruption (`StoreCorrupt`) instead of the distinct,
/// actionable `StoreFull` (root design: disk full, lock contention and
/// corrupt state need distinct actionable results).
#[test]
fn disk_full_send_fails_explicitly_without_acceptance_and_retry_commits_once() {
    let m = Matrix::new();
    let (s, _r, thread) = m.pair();
    let first = m.send(&s, &thread, "kept history", &[], "send-0").unwrap();
    let ops = m.count("SELECT count(*) FROM operations", []);
    let big = "z".repeat(48 * 1024);
    let error = {
        let _full = Failpoint::connection("store.writer.acquired", m.scope(), |conn| {
            let pages: i64 = conn
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .unwrap();
            conn.pragma_update(None, "max_page_count", pages).unwrap();
        });
        m.send(&s, &thread, &big, &[], "send-big").unwrap_err()
    };
    assert_eq!(error.code, ErrorCode::StoreFull, "{error:?}");
    assert!(error.detail.contains("full"), "{error:?}");
    assert!(error.detail.contains("free disk space"), "{error:?}");
    assert_eq!(m.message_rows(), 1);
    assert_eq!(m.count("SELECT count(*) FROM operations", []), ops);
    assert_eq!(m.pending("r"), vec![first.clone()]);
    let _space = Failpoint::connection("store.writer.acquired", m.scope(), |conn| {
        conn.pragma_update(None, "max_page_count", 1_073_741_823i64)
            .unwrap();
    });
    let sent = m.send(&s, &thread, &big, &[], "send-big").unwrap();
    assert_eq!(m.send(&s, &thread, &big, &[], "send-big").unwrap(), sent);
    assert_eq!(m.message_rows(), 2);
    assert_eq!(m.pending("r"), vec![first, sent]);
}

/// Storage fault: another process holds the database write lock.
/// Kills: an operation reported accepted (or durably half-applied) while the
/// writer could not obtain the lock.
#[test]
fn locked_database_send_fails_busy_without_acceptance() {
    let m = Matrix::new();
    let (s, _r, thread) = m.pair();
    let blocker = m.db();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let error = m.send(&s, &thread, "hello", &[], "send-1").unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreBusy, "{error:?}");
    blocker.execute_batch("ROLLBACK").unwrap();
    drop(blocker);
    assert_eq!(m.message_rows(), 0);
    assert_eq!(m.count("SELECT count(*) FROM send_preparations", []), 0);
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    assert_eq!(m.message_rows(), 1);
    assert_eq!(m.pending("r"), vec![sent]);
}

/// Storage fault: corrupt bytes, altered schema and an unknown future schema
/// version, opened through the production store factory.
/// Kills: silently migrating, re-initializing or rewriting a database the
/// store does not understand (history overwritten), or accepting it.
#[test]
fn corrupt_or_unknown_schema_fails_explicitly_and_never_rewrites_history() {
    use sha2::{Digest, Sha256};
    let m = Matrix::new();
    let (s, _r, thread) = m.pair();
    m.send(&s, &thread, "history", &[], "send-1").unwrap();
    let Matrix {
        store,
        service,
        ports,
        db_path,
        clock,
        instance,
        _scratch,
        ..
    } = m;
    drop(service);
    drop(ports);
    drop(store);
    {
        let db = Connection::open(&db_path).unwrap();
        db.pragma_update(None, "journal_mode", "DELETE").unwrap();
    }
    let hash = |path: &PathBuf| Sha256::digest(std::fs::read(path).unwrap()).to_vec();
    type Variant = (&'static str, fn(&PathBuf), &'static [ErrorCode]);
    let variants: [Variant; 3] = [
        (
            "unknown-version",
            |path| {
                Connection::open(path)
                    .unwrap()
                    .pragma_update(None, "user_version", 999)
                    .unwrap();
            },
            &[ErrorCode::IncompatibleSchema],
        ),
        (
            "altered-schema",
            |path| {
                Connection::open(path)
                    .unwrap()
                    .execute_batch("DROP INDEX retirements_failed_pending")
                    .unwrap();
            },
            &[ErrorCode::IncompatibleSchema, ErrorCode::StoreCorrupt],
        ),
        (
            "corrupt-bytes",
            |path| {
                use std::io::{Seek, SeekFrom, Write};
                let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
                let page = 4096u64;
                file.seek(SeekFrom::Start(page * 2 + 8)).unwrap();
                file.write_all(&[0xA5; 512]).unwrap();
            },
            &[ErrorCode::StoreCorrupt],
        ),
    ];
    for (label, damage, expected) in variants {
        let path = db_path.with_file_name(format!("{label}.db"));
        std::fs::copy(&db_path, &path).unwrap();
        damage(&path);
        let before = hash(&path);
        let clock: Arc<dyn Clock> = clock.clone();
        let error = match SqliteStore::new(
            StoreContext::new(path.clone(), clock),
            instance.clone(),
            StoreSettings::default(),
        ) {
            Ok(_) => panic!("{label}: damaged store accepted"),
            Err(error) => error,
        };
        assert!(expected.contains(&error.code), "{label}: {error:?}");
        assert_eq!(hash(&path), before, "{label}: file rewritten");
        if label != "corrupt-bytes" {
            let raw = Connection::open(&path).unwrap();
            let history: i64 = raw
                .query_row(
                    "SELECT count(*) FROM messages WHERE body='history'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(history, 1, "{label}: history retained");
        }
    }
}

/// Fake host for the observation lane: the socket is denied (and lifecycle
/// events are lost) until `reconnect`, then complete coherent snapshots are
/// served.
struct FlakyHost {
    connected: std::sync::atomic::AtomicBool,
    /// Answers with an incomplete enumeration: evidence the host view is not
    /// coherently known, so the capture still invalidates (unlike a denied
    /// socket, which only freezes state, ht-yms).
    partial: std::sync::atomic::AtomicBool,
    sequence: AtomicU64,
    /// Target address and, once an agent runs there, its execution.
    targets: Mutex<Vec<(String, Option<String>)>>,
    clock: Arc<ManualClock>,
    shape: HostShape,
}

/// What a host double reports per target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HostShape {
    /// Exactly what the production Herdr 0.9.1 adapter reports
    /// (`NativeCli::observation`, `src/host/native.rs`): the terminal and the
    /// verified server incarnation only; occupant None, UI, occupancy and
    /// execution Unknown, whatever runs in the pane.
    ProductionAdapter,
    /// A host that also proves the top-level occupant and its execution.
    VerifiedOccupant,
}
impl FlakyHost {
    fn observation(
        &self,
        target: &str,
        execution: Option<&str>,
        provenance: crate::ports::ObservationProvenance,
        sequence: u64,
    ) -> crate::ports::HostObservation {
        use crate::ports::*;
        let now = self.clock.monotonic_now();
        let execution = match self.shape {
            HostShape::ProductionAdapter => None,
            HostShape::VerifiedOccupant => execution,
        };
        HostObservation {
            focused: false,
            target: HostTargetId::new(target),
            host_boot: HostBootId::new("b"),
            epoch: 1,
            generation: 1,
            observed_at_utc: self.clock.utc_now(),
            observed_at_mono: now,
            provenance,
            occupant: execution.map(|execution| NativeOccupant {
                harness: Harness::Codex,
                session: NativeSessionId::new(format!("plugin_context:{target}")),
                execution: ExecutionId::new(execution),
                is_top_level: true,
            }),
            ui: match self.shape {
                HostShape::ProductionAdapter => HostUiState::Unknown,
                HostShape::VerifiedOccupant => HostUiState::Idle,
            },
            terminal: Some(TerminalId::new(format!("term-{target}"))),
            occupancy: match (self.shape, execution) {
                (HostShape::ProductionAdapter, _) => StructuralOccupancy::Unknown,
                (HostShape::VerifiedOccupant, Some(_)) => StructuralOccupancy::Occupied,
                (HostShape::VerifiedOccupant, None) => StructuralOccupancy::EmptyShell,
            },
            incarnation: IncarnationEvidence::Verified {
                identity: "inc".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            execution: execution
                .map(|execution| ExecutionEvidence::Verified {
                    execution: ExecutionId::new(execution),
                    evidence_kind: EvidenceKind::NativeInvocation,
                })
                .unwrap_or(ExecutionEvidence::Unknown),
            call_id: HostCallId::new(format!("call-{sequence}")),
            connection_epoch: 1,
            observation_sequence: sequence,
            started_at_mono: now,
            completed_at_mono: now,
        }
    }
}
impl crate::ports::HostPort for FlakyHost {
    fn native_launch_capability(&self) -> crate::ports::NativeLaunchCapability {
        crate::ports::NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<crate::ports::HostObservation, ApiError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(host_denied());
        }
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        let targets = self.targets.lock().unwrap();
        let (address, execution) = targets
            .iter()
            .find(|(address, _)| address == target.as_str())
            .ok_or_else(host_denied)?;
        Ok(self.observation(
            address,
            execution.as_deref(),
            crate::ports::ObservationProvenance::FreshCurrentTarget,
            sequence,
        ))
    }
    fn enumerate_targets(
        &self,
        _: &HostCallContext,
    ) -> Result<crate::ports::HostSnapshot, ApiError> {
        use crate::ports::*;
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        if !self.connected.load(Ordering::SeqCst) {
            return Err(host_denied());
        }
        Ok(HostSnapshot {
            boot: HostBootId::new("b"),
            epoch: 1,
            observation_sequence: sequence,
            complete: !self.partial.load(Ordering::SeqCst),
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: "inc".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets: self
                .targets
                .lock()
                .unwrap()
                .iter()
                .map(|(address, execution)| {
                    self.observation(
                        address,
                        execution.as_deref(),
                        ObservationProvenance::CoherentEnumeration,
                        sequence,
                    )
                })
                .collect(),
        })
    }
    fn safe_wake_target(
        &self,
        _: &SeatId,
        _: &crate::ports::HostObservation,
    ) -> Option<crate::ports::SafeWakeTarget> {
        None
    }
    fn submit_prompt(
        &self,
        _: &crate::ports::SafeWakeTarget,
        _: &str,
        _: &HostCallContext,
    ) -> Result<crate::ports::PromptOutcome, ApiError> {
        Err(host_denied())
    }
    fn pane_agent_state(
        &self,
        _target: &crate::ports::SafeWakeTarget,
        _context: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }

    fn launch_native(
        &self,
        _: crate::ports::NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<crate::ports::NativeLaunchOutcome, ApiError> {
        Err(host_denied())
    }
    fn send_submit_key(
        &self,
        _: &crate::ports::SafeWakeTarget,
        _: &crate::ports::HostCallContext,
    ) -> Result<(), crate::protocol::results::ApiError> {
        Ok(())
    }
}
fn host_denied() -> ApiError {
    ApiError::host_unavailable("socket denied")
}

impl Matrix {
    /// One production observation-worker turn sequence: capture if due,
    /// then reconcile every page. As in `start_observation_worker`, a
    /// reconcile error is recorded and the continuation dropped until the
    /// next due capture.
    fn capture_and_reconcile(
        &self,
        identity: &crate::identity::repair::OrdinaryIdentity,
    ) -> (
        crate::identity::reconcile::ObservationOutcome,
        u32,
        Option<ApiError>,
    ) {
        let outcome = identity
            .capture_if_due(&self.ports, &self.budget(), &self.budget())
            .unwrap()
            .expect("capture due");
        let (mut after, mut high, mut retirements) = (0, None, 0u32);
        loop {
            match identity.reconcile_page(&self.ports, &outcome, after, high, &self.budget()) {
                Ok(progress) => {
                    retirements += u32::from(progress.retirements_started);
                    high = Some(progress.high_water_ordinal);
                    match progress.next_after_ordinal {
                        Some(next) => after = next,
                        None => return (outcome, retirements, None),
                    }
                }
                Err(error) => return (outcome, retirements, Some(error)),
            }
        }
    }

    fn seat_states(&self) -> String {
        self.db()
            .query_row(
                "SELECT group_concat(state||':'||COALESCE(unresolved_reason,'-'),' ') FROM (SELECT * FROM seats ORDER BY target_id)",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
}

/// A composed daemon over `FlakyHost` with two structurally resolved,
/// cooperatively checked-in seats in one thread and one pending receipt.
struct FlakyPair {
    host: Arc<FlakyHost>,
    identity: Arc<crate::identity::repair::OrdinaryIdentity>,
    s: CallerClaim,
    r: CallerClaim,
    sent: MessageId,
}

impl Matrix {
    /// Compose the production service over the flaky host observation lane.
    fn compose_flaky(
        &mut self,
        host: &Arc<FlakyHost>,
    ) -> Arc<crate::identity::repair::OrdinaryIdentity> {
        let identity = Arc::new(crate::identity::repair::OrdinaryIdentity::new(
            self.instance.clone(),
            self.store.clone(),
            host.clone(),
            self.clock.clone(),
            Arc::new(FairWriter::new(32)),
        ));
        let port: Arc<dyn StorePort> = self.store.clone();
        self.service = DomainService::with_identity(
            self.instance.clone(),
            port,
            self.clock.clone(),
            identity.clone(),
        )
        .with_operator_owner(OWNER)
        .with_cooperative_owner(OWNER, Arc::new(FairWriter::new(32)));
        identity
    }

    /// Baseline publication, production structural resolution of two panes,
    /// cooperative lifecycle check-ins, one thread and one pending receipt.
    fn flaky_pair(&mut self, shape: HostShape) -> FlakyPair {
        use crate::identity::reconcile::ObservationOutcome;
        use crate::protocol::commands::ResolveSeat;
        let host = Arc::new(FlakyHost {
            connected: std::sync::atomic::AtomicBool::new(true),
            partial: std::sync::atomic::AtomicBool::new(false),
            sequence: AtomicU64::new(10),
            targets: Mutex::new(vec![("pane-s".into(), None), ("pane-r".into(), None)]),
            clock: self.clock.clone(),
            shape,
        });
        let identity = self.compose_flaky(&host);
        let (baseline, retirements, error) = self.capture_and_reconcile(&identity);
        assert!(
            matches!(baseline, ObservationOutcome::Published(_)),
            "{baseline:?}"
        );
        assert_eq!((retirements, error), (0, None));
        let mut claims = Vec::new();
        for pane in ["pane-s", "pane-r"] {
            let CommandResult::SeatResolved(seat) = self
                .handle(Command::ResolveSeat(ResolveSeat {
                    target: HostTargetId::new(pane),
                    operation: OperationId::new(format!("resolve-{pane}")),
                }))
                .unwrap()
            else {
                panic!("resolution")
            };
            let n = self.executions.fetch_add(1, Ordering::SeqCst);
            let execution = format!("00000000-0000-4000-8000-{n:012}");
            host.targets
                .lock()
                .unwrap()
                .iter_mut()
                .find(|(address, _)| address == pane)
                .unwrap()
                .1 = Some(execution.clone());
            let claim = CallerClaim {
                instance: self.instance.clone(),
                seat: seat.clone(),
                binding_generation: 0,
                role: CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new(format!("plugin_context:{pane}")),
                execution: ExecutionId::new(execution),
                target: HostTargetId::new(pane),
            };
            claims.push(self.lifecycle(claim, &format!("checkin-{pane}")));
        }
        let (s, r) = (claims[0].clone(), claims[1].clone());
        let thread = self.create_thread(&s, "thread");
        self.join(&s, &thread, &r);
        let sent = self.send(&s, &thread, "hello", &[], "send-1").unwrap();
        FlakyPair {
            host,
            identity,
            s,
            r,
            sent,
        }
    }

    /// The reconfirmation evidence stored on each seat's latest binding.
    fn binding_evidence(&self) -> Vec<(String, Option<String>, Option<String>)> {
        let db = self.db();
        let mut stmt = db
            .prepare("SELECT b.target_id,b.terminal_id,b.incarnation FROM occupant_bindings b WHERE b.ordinal=(SELECT MAX(x.ordinal) FROM occupant_bindings x WHERE x.seat_id=b.seat_id) ORDER BY b.target_id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// An incomplete enumeration at the next periodic capture (every event
    /// lost): fail-closed invalidation that never retires or settles anything.
    fn invalidate_host(&self, pair: &FlakyPair) {
        self.invalidate_host_expecting(
            pair,
            "unresolved:host_invalidation unresolved:host_invalidation",
        );
    }

    fn invalidate_host_expecting(&self, pair: &FlakyPair, states: &str) {
        use crate::identity::reconcile::ObservationOutcome;
        pair.host.partial.store(true, Ordering::SeqCst);
        self.clock.mono.fetch_add(5_000, Ordering::SeqCst);
        let (denied, retirements, error) = self.capture_and_reconcile(&pair.identity);
        assert!(
            matches!(
                &denied,
                ObservationOutcome::Invalidated {
                    reason: crate::ports::HostInvalidationReason::PartialEnumeration,
                    ..
                }
            ),
            "{denied:?}"
        );
        assert_eq!((retirements, error), (0, None));
        assert_eq!(self.seat_states(), states);
        self.assert_nothing_retired_or_settled(pair);
    }

    fn assert_nothing_retired_or_settled(&self, pair: &FlakyPair) {
        let r_seat = pair.r.seat.as_str();
        assert_eq!(
            self.count(
                "SELECT count(*) FROM seats WHERE state IN ('retiring','retired') OR retired_at IS NOT NULL",
                [],
            ),
            0
        );
        assert_eq!(self.count("SELECT count(*) FROM retirements", []), 0);
        assert_eq!(self.receipt_state(&pair.sent, r_seat), "pending");
        assert_eq!(self.ack_events(r_seat), 0);
        assert_eq!(self.pending(r_seat), vec![pair.sent.clone()]);
    }

    /// Periodic captures after reconnect: each must publish and reconcile
    /// every page without error, retirement or settlement.
    fn reconnect_and_capture(&self, pair: &FlakyPair, turns: usize) {
        use crate::identity::reconcile::ObservationOutcome;
        pair.host.connected.store(true, Ordering::SeqCst);
        pair.host.partial.store(false, Ordering::SeqCst);
        for turn in 0..turns {
            self.clock.mono.fetch_add(5_000, Ordering::SeqCst);
            let (published, retirements, error) = self.capture_and_reconcile(&pair.identity);
            assert!(
                matches!(published, ObservationOutcome::Published(_)),
                "turn {turn}: {published:?}"
            );
            assert_eq!(error, None, "turn {turn}: reconcile error");
            assert_eq!(retirements, 0, "turn {turn}");
            self.assert_nothing_retired_or_settled(pair);
        }
    }
}

/// The shipped CLI for one seat's pane: `run_selected`, its context gate and
/// the seat's private lifecycle-context journal, over this composed service.
struct CliSeat {
    paths: crate::daemon::paths::InstancePaths,
    instance: uuid::Uuid,
    selection: crate::cli::commands::CooperativeSelection,
}

struct MatrixClient<'a>(&'a Matrix);
impl crate::ports::LocalClient for MatrixClient<'_> {
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.0.handle(command)
    }
    fn call_with_output(
        &self,
        command: Command,
        _: &crate::protocol::output::OutputSpec,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.0.handle(command)
    }
}

impl Matrix {
    fn cli_seat(&self, claim: &CallerClaim) -> CliSeat {
        let root = self._scratch.0.join("cli");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let runtime = crate::daemon::paths::RuntimeContext::explicit(
            root.join("state"),
            root.join("host.sock"),
            None,
        )
        .unwrap();
        CliSeat {
            paths: crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap(),
            instance: uuid::Uuid::parse_str(&self.instance).expect("UUID instance"),
            selection: crate::cli::commands::CooperativeSelection {
                seat: claim.seat.clone(),
                target: claim.target.clone(),
                harness: crate::harness::context::Harness::Codex,
                role: crate::harness::context::Role::TopLevel,
            },
        }
    }

    /// Run one argv through the shipped CLI for this seat's pane.
    fn cli(&self, seat: &CliSeat, argv: &[&str]) -> Result<Vec<u8>, crate::cli::RunError> {
        let mut full = vec!["herdr-threads"];
        full.extend_from_slice(argv);
        let parsed = crate::cli::commands::parse_argv(full).unwrap();
        let mut output = Vec::new();
        crate::cli::run_selected(
            parsed,
            &seat.selection,
            &seat.paths,
            seat.instance,
            &MatrixClient(self),
            self.clock.as_ref(),
            &mut output,
        )
        .map(|()| output)
    }

    /// The claim the CLI's private context journal currently holds.
    fn cli_context(&self, seat: &CliSeat) -> CallerClaim {
        let contexts = crate::harness::context::ContextJournal::open(
            &crate::cli::seat_context_dir(&seat.paths, seat.selection.seat.as_str()).unwrap(),
            seat.instance,
            seat.selection.seat.as_str(),
            Duration::from_secs(1),
        )
        .unwrap();
        crate::harness::bridge::caller_claim(&contexts.current().unwrap().expect("context"))
            .unwrap()
    }

    fn seat_generation(&self, seat: &SeatId) -> i64 {
        self.count("SELECT generation FROM seats WHERE id=?1", [seat.as_str()])
    }
}

fn cli_refusal(result: Result<Vec<u8>, crate::cli::RunError>) -> ApiError {
    match result {
        Err(crate::cli::RunError::Api(error)) => error,
        other => panic!("expected a CLI API refusal, got {other:?}"),
    }
}

/// ht-yms (TRUST-POLICY C4): Herdr not answering is missing evidence, not
/// evidence of change. A denied socket writes no invalidation: both seats
/// stay resolved at their generation with their bindings open, the
/// recipient's pre-outage context still settles its receipt, and reconnect
/// needs no repair. Kills: invalidating (unresolving seats, ending bindings)
/// on a host that merely did not answer.
#[test]
fn socket_denial_freezes_seats_and_bindings_until_reconnect() {
    use crate::identity::reconcile::ObservationOutcome;
    let mut m = Matrix::with_uuid_instance();
    let pair = m.flaky_pair(HostShape::ProductionAdapter);
    let states = m.seat_states();
    let generation = m.seat_generation(&pair.r.seat);
    let open_bindings = || {
        m.count(
            "SELECT count(*) FROM occupant_bindings WHERE ended_at IS NULL",
            [],
        )
    };
    let bindings = open_bindings();
    pair.host.connected.store(false, Ordering::SeqCst);
    for _ in 0..2 {
        m.clock.mono.fetch_add(5_000, Ordering::SeqCst);
        let outcome = pair
            .identity
            .capture_if_due(&m.ports, &m.budget(), &m.budget())
            .unwrap()
            .expect("capture due");
        assert!(
            matches!(
                &outcome,
                ObservationOutcome::Frozen {
                    reason: crate::ports::HostInvalidationReason::HostUnavailable,
                    cause: Some(e),
                } if e.code == ErrorCode::HostUnavailable
            ),
            "{outcome:?}"
        );
        assert_eq!(m.seat_states(), states);
        assert_eq!(m.seat_generation(&pair.r.seat), generation);
        assert_eq!(open_bindings(), bindings);
        assert_eq!(
            m.count("SELECT invalidation_revision FROM host_instances", []),
            0
        );
    }
    m.assert_nothing_retired_or_settled(&pair);
    // Frozen, not unavailable: the pre-outage context still settles its
    // receipt (a store decision that needs no host read).
    assert_eq!(
        m.ack(&pair.r, &[&pair.sent], "ack-during-outage")
            .unwrap()
            .acknowledged,
        vec![pair.sent.clone()]
    );
    pair.host.connected.store(true, Ordering::SeqCst);
    m.clock.mono.fetch_add(5_000, Ordering::SeqCst);
    let (published, retirements, error) = m.capture_and_reconcile(&pair.identity);
    assert!(
        matches!(published, ObservationOutcome::Published(_)),
        "{published:?}"
    );
    assert_eq!((retirements, error), (0, None));
    assert_eq!(m.seat_states(), states);
    assert_eq!(m.seat_generation(&pair.r.seat), generation);
}

/// Row 12. Boundary: an incomplete host capture (evidence the view is not
/// coherently known; a denied socket only freezes, see
/// `socket_denial_freezes_seats_and_bindings_until_reconnect`) and lifecycle
/// events lost, then reconnect, over a host double with the **production adapter's**
/// observation shape (occupant None; UI, occupancy and execution Unknown).
/// Seats are allocated by production resolution over a real baseline; the
/// recipient's agent registers through the shipped CLI (context gate plus
/// private context journal) exactly as its lifecycle hook does.
/// Kills: inferring retirement or settlement from a host error; a binding
/// stored without the verified observation's terminal/incarnation; any
/// reconcile error after reconnect; `plan_page` requiring occupancy or
/// execution evidence to reconfirm a host-invalidated seat (ht-4is.11.1
/// fix1 B1: both seats then stay `unresolved:host_invalidation` forever on
/// the real adapter); and the CLI context gate refusing a fresh lifecycle
/// check-in after the invalidation's generation bump (fix1 S2), while it
/// must still refuse a stale-context mutation.
#[test]
fn incomplete_capture_and_lost_events_recover_by_snapshot_without_retirement() {
    let mut m = Matrix::with_uuid_instance();
    let pair = m.flaky_pair(HostShape::ProductionAdapter);
    let cli = m.cli_seat(&pair.r);
    // The recipient's first lifecycle hook, through the CLI.
    m.cli(&cli, &["check-in", "--lifecycle-event", "hook-r-1"])
        .unwrap();
    let r_before = m.cli_context(&cli);
    assert_eq!(r_before.seat, pair.r.seat);
    // Each binding carries the evidence of the observation that proved it.
    assert_eq!(
        m.binding_evidence(),
        vec![
            (
                "pane-r".to_string(),
                Some("term-pane-r".to_string()),
                Some("inc".to_string())
            ),
            (
                "pane-s".to_string(),
                Some("term-pane-s".to_string()),
                Some("inc".to_string())
            ),
        ]
    );
    let operator_ops = || {
        m.count(
            "SELECT count(*) FROM operations WHERE actor_scope LIKE 'operator:%'",
            [],
        )
    };
    let operator_before = operator_ops();
    m.invalidate_host(&pair);
    let invalidated_generation = m.seat_generation(&pair.r.seat);
    assert!(invalidated_generation > r_before.binding_generation as i64);
    // Reconnect: the first complete production-shaped snapshot reconfirms
    // both seats structurally with no operator action; repeated captures
    // (no event ever arrives) stay clean.
    m.reconnect_and_capture(&pair, 1);
    assert_eq!(m.seat_states(), "resolved:- resolved:-");
    m.reconnect_and_capture(&pair, 2);
    assert_eq!(m.seat_states(), "resolved:- resolved:-");
    assert_eq!(operator_ops(), operator_before, "no operator repair");
    assert_eq!(m.seat_generation(&pair.r.seat), invalidated_generation);
    // The invalidation fenced the pre-denial contexts; nothing settles
    // through them, at the service or through the CLI gate.
    let error = m.ack(&r_before, &[&pair.sent], "ack-stale").unwrap_err();
    assert_eq!(error.code, ErrorCode::CallerUnverified, "{error:?}");
    let refused = cli_refusal(m.cli(&cli, &["ack", pair.sent.as_str()]));
    assert_eq!(
        (refused.code, refused.detail.as_str()),
        (
            ErrorCode::TargetUnresolved,
            "local context differs from current service mapping"
        )
    );
    // A `Current` check-in only continues the local context: refused too.
    let refused = cli_refusal(m.cli(&cli, &["check-in"]));
    assert_eq!(refused.code, ErrorCode::TargetUnresolved, "{refused:?}");
    m.assert_nothing_retired_or_settled(&pair);
    // The agent's next lifecycle hook registers through the CLI gate at the
    // current generation, and the successor context settles the receipt.
    m.cli(&cli, &["check-in", "--lifecycle-event", "hook-r-2"])
        .unwrap();
    let r = m.cli_context(&cli);
    assert_eq!(r.seat, pair.r.seat);
    assert_ne!(r.execution, r_before.execution);
    assert!(r.binding_generation as i64 > invalidated_generation);
    m.cli(&cli, &["ack", pair.sent.as_str()]).unwrap();
    assert_eq!(m.receipt_state(&pair.sent, pair.r.seat.as_str()), "acked");
    assert_eq!(m.ack_events(r.seat.as_str()), 1);
    // A replay of the first hook's event is still exact replay only: it
    // renders its recorded result and registers nothing new.
    let generation = m.seat_generation(&pair.r.seat);
    m.cli(&cli, &["check-in", "--lifecycle-event", "hook-r-1"])
        .unwrap();
    assert_eq!(m.seat_generation(&pair.r.seat), generation);
    assert_eq!(m.cli_context(&cli), r);
    let mut next = pair.s.clone();
    next.execution = ExecutionId::new("00000000-0000-4000-8000-0000000000fd");
    let s = m.lifecycle(next, "checkin-s-after-reconnect");
    assert_eq!(s.seat, pair.s.seat);
    m.reconnect_and_capture_settled(&pair);
}

/// Row 12 variant: a host that also proves the top-level occupant keeps the
/// unchanged execution-evidence reconfirmation.
/// Kills: the structural bridge displacing verified-execution reconfirmation.
#[test]
fn incomplete_capture_recovers_with_verified_occupant_host() {
    let mut m = Matrix::new();
    let pair = m.flaky_pair(HostShape::VerifiedOccupant);
    m.invalidate_host(&pair);
    m.reconnect_and_capture(&pair, 1);
    assert_eq!(m.seat_states(), "resolved:- resolved:-");
    let mut next = pair.r.clone();
    next.execution = ExecutionId::new("00000000-0000-4000-8000-0000000000fe");
    pair.host.targets.lock().unwrap()[1].1 = Some(next.execution.as_str().into());
    let r = m.lifecycle(next, "checkin-r-after-reconnect");
    assert_eq!(
        m.ack(&r, &[&pair.sent], "ack-1").unwrap().acknowledged,
        vec![pair.sent.clone()]
    );
    m.reconnect_and_capture_settled(&pair);
}

impl Matrix {
    /// One more clean periodic turn after the agents re-registered.
    fn reconnect_and_capture_settled(&self, pair: &FlakyPair) {
        self.clock.mono.fetch_add(5_000, Ordering::SeqCst);
        let (_, retirements, error) = self.capture_and_reconcile(&pair.identity);
        assert_eq!((retirements, error), (0, None));
        assert_eq!(self.seat_states(), "resolved:- resolved:-");
    }

    /// Model a binding written before the evidence invariant (raw
    /// connection: no guard) for the given pane, or every pane.
    fn strip_binding_evidence(&self, target: Option<&str>) {
        self.db()
            .execute(
                "UPDATE occupant_bindings SET terminal_id=NULL,incarnation=NULL WHERE ?1 IS NULL OR target_id=?1",
                [target],
            )
            .unwrap();
    }
}

/// Migration boundary: a store written before the binding-evidence invariant
/// (cooperative bindings without terminal/incarnation) whose seats are stuck
/// unresolved after a host denial, over the production adapter's shape.
/// Without a restart they stay unresolved, fail-closed and without a
/// reconcile error; the restarted writer backfills evidence from each seat's
/// own verified structural proof and the next snapshot recovers them.
/// Kills: dropping the startup backfill; the structural bridge accepting a
/// binding with no stored evidence.
#[test]
fn legacy_unconfirmable_bindings_heal_after_writer_restart_backfill() {
    let mut m = Matrix::new();
    let pair = m.flaky_pair(HostShape::ProductionAdapter);
    m.invalidate_host(&pair);
    m.strip_binding_evidence(None);
    m.reconnect_and_capture(&pair, 2);
    assert_eq!(
        m.seat_states(),
        "unresolved:host_invalidation unresolved:host_invalidation"
    );
    // Daemon restart: the writer's startup verification repairs the rows
    // and reports how many it repaired.
    m.restart();
    assert_eq!(
        m.store.binding_evidence_startup(),
        Some(crate::ports::BindingEvidenceStartup {
            backfilled: 2,
            still_lacking: 0,
        })
    );
    let identity = m.compose_flaky(&pair.host);
    let pair = FlakyPair { identity, ..pair };
    assert_eq!(
        m.binding_evidence(),
        vec![
            (
                "pane-r".to_string(),
                Some("term-pane-r".to_string()),
                Some("inc".to_string())
            ),
            (
                "pane-s".to_string(),
                Some("term-pane-s".to_string()),
                Some("inc".to_string())
            ),
        ]
    );
    m.reconnect_and_capture(&pair, 2);
    assert_eq!(m.seat_states(), "resolved:- resolved:-");
}

/// Boundary: reconnect when the earlier seat cannot be reconfirmed (its
/// latest binding has no reconfirmation evidence), over the production
/// adapter's shape.
/// Kills: treating "keep this already-unresolved seat unresolved" as a stale
/// plan. Before that fix every turn failed with CursorStale at that seat and
/// every later seat in the page (here the receipt recipient) stayed
/// unresolved forever.
#[test]
fn unreconfirmable_seat_stays_unresolved_without_blocking_later_seats() {
    let mut m = Matrix::new();
    let pair = m.flaky_pair(HostShape::ProductionAdapter);
    m.invalidate_host(&pair);
    m.strip_binding_evidence(Some("pane-s"));
    m.reconnect_and_capture(&pair, 3);
    assert_eq!(
        m.seat_states(),
        "resolved:- unresolved:host_invalidation",
        "pane-r reconfirmed; pane-s stays fail-closed"
    );
    // Restart backfills pane-s from its own structural proof: a later
    // snapshot reconfirms it, still without an operator.
    m.restart();
    let identity = m.compose_flaky(&pair.host);
    let pair = FlakyPair { identity, ..pair };
    m.reconnect_and_capture(&pair, 1);
    assert_eq!(m.seat_states(), "resolved:- resolved:-");
}

/// Boundary: explicit operator repair (`OperatorRebind`) while a receipt is
/// pending (row 9 against an actual repair, not a fresh check-in). The
/// recipient's seat cannot be reconfirmed automatically (no stored binding
/// evidence), so only a repair restores it; its agent then registers
/// through the shipped CLI gate.
/// Kills: a repair that does not bump/fence the seat binding generation, so a
/// pre-repair context could still settle; and a successor that cannot
/// register or settle through the CLI after the repair.
#[test]
fn operator_repair_invalidates_pre_repair_context() {
    let mut m = Matrix::with_uuid_instance();
    let pair = m.flaky_pair(HostShape::ProductionAdapter);
    let cli = m.cli_seat(&pair.r);
    m.cli(&cli, &["check-in", "--lifecycle-event", "hook-r-1"])
        .unwrap();
    let r_before = m.cli_context(&cli);
    m.invalidate_host(&pair);
    m.strip_binding_evidence(Some("pane-r"));
    m.reconnect_and_capture(&pair, 1);
    assert_eq!(m.seat_states(), "unresolved:host_invalidation resolved:-");
    let generation_before = m.seat_generation(&pair.r.seat);
    let CommandResult::OperatorRebound(rebound) = m
        .handle(Command::OperatorRebind(
            crate::protocol::commands::OperatorRebind {
                seat: pair.r.seat.clone(),
                target: pair.r.target.clone(),
                operation: OperationId::new("repair-r"),
            },
        ))
        .unwrap()
    else {
        panic!("operator repair")
    };
    assert_eq!(rebound, pair.r.seat);
    assert!(m.seat_generation(&pair.r.seat) > generation_before);
    // The pre-repair context is fenced, at the service and at the CLI gate.
    let error = m
        .ack(&r_before, &[&pair.sent], "ack-pre-repair")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::CallerUnverified, "{error:?}");
    assert_eq!(error.detail, "cooperative binding generation changed");
    let refused = cli_refusal(m.cli(&cli, &["ack", pair.sent.as_str()]));
    assert_eq!(refused.code, ErrorCode::TargetUnresolved, "{refused:?}");
    assert_eq!(m.receipt_state(&pair.sent, pair.r.seat.as_str()), "pending");
    assert_eq!(m.ack_events(pair.r.seat.as_str()), 0);
    assert_eq!(m.pending(pair.r.seat.as_str()), vec![pair.sent.clone()]);
    // The pane's next lifecycle hook registers against the repaired seat.
    m.cli(
        &cli,
        &["check-in", "--lifecycle-event", "hook-r-after-repair"],
    )
    .unwrap();
    let r = m.cli_context(&cli);
    assert!(r.binding_generation as i64 > generation_before);
    m.cli(&cli, &["ack", pair.sent.as_str()]).unwrap();
    assert_eq!(m.ack_events(r.seat.as_str()), 1);
}

/// Row 12 variant, wave-2 fix2 (b) (fix2 review S1): a pane resolved before
/// its agent starts and never registered, over the **production adapter's**
/// observation shape. After one transient socket denial the seat must be
/// reconfirmed from its own verified structural proof, and its agent's first
/// fresh lifecycle check-in through the shipped CLI must register at the
/// current generation, with no operator action. A check-in that arrives
/// while the seat is still unresolved is refused, but never silently: one
/// bounded line (the stderr line `main` prints) that names the repair, and
/// the unresolved-seat channel Health and doctor render names the seat.
/// Kills: `plan_page` without the never-registered structural bridge (the
/// seat stays `unresolved:host_invalidation` and the hook is refused on
/// every attempt), the store prepare refusing a seat with no binding, the
/// CLI's generic unresolved refusal, and a missing unresolved summary.
#[test]
fn resolved_never_registered_seat_registers_on_first_lifecycle_check_in_after_reconnect() {
    use crate::identity::reconcile::ObservationOutcome;
    use crate::ports::{StorePort, UnresolvedReason};
    use crate::protocol::commands::ResolveSeat;
    let mut m = Matrix::with_uuid_instance();
    let pair = m.flaky_pair(HostShape::ProductionAdapter);
    // A third pane appears and is resolved before any agent runs there.
    pair.host
        .targets
        .lock()
        .unwrap()
        .push(("pane-u".into(), None));
    m.clock.mono.fetch_add(5_000, Ordering::SeqCst);
    let (published, retirements, error) = m.capture_and_reconcile(&pair.identity);
    assert!(
        matches!(published, ObservationOutcome::Published(_)),
        "{published:?}"
    );
    assert_eq!((retirements, error), (0, None));
    let CommandResult::SeatResolved(seat_u) = m
        .handle(Command::ResolveSeat(ResolveSeat {
            target: HostTargetId::new("pane-u"),
            operation: OperationId::new("resolve-pane-u"),
        }))
        .unwrap()
    else {
        panic!("resolution")
    };
    let bindings_u = || {
        m.count(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id=?1",
            [seat_u.as_str()],
        )
    };
    assert_eq!(bindings_u(), 0, "resolved but never registered");
    let u = CallerClaim {
        instance: m.instance.clone(),
        seat: seat_u.clone(),
        binding_generation: 0,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("plugin_context:pane-u"),
        execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
        target: HostTargetId::new("pane-u"),
    };
    let cli = m.cli_seat(&u);
    let operator_ops = || {
        m.count(
            "SELECT count(*) FROM operations WHERE actor_scope LIKE 'operator:%'",
            [],
        )
    };
    let operator_before = operator_ops();
    m.invalidate_host_expecting(
        &pair,
        "unresolved:host_invalidation unresolved:host_invalidation unresolved:host_invalidation",
    );
    let generation_u = m.seat_generation(&seat_u);
    // The agent's first hook fires while the seat is still unresolved: a
    // refusal, surfaced as one bounded line that names the repair.
    let refused = m
        .cli(&cli, &["check-in", "--lifecycle-event", "hook-u-early"])
        .unwrap_err();
    let line = refused.to_string();
    let expected = format!(
        "seat {seat} is unresolved, so nothing was registered or changed; a lifecycle check-in \
         registers once a coherent host snapshot reconfirms pane pane-u; if the seat stays \
         unresolved, `herdr-threads doctor` lists it: repair with `seat rebind {seat} --pane \
         pane-u --operator` (target_unresolved)",
        seat = seat_u.as_str()
    );
    assert_eq!(line, expected);
    assert!(!line.contains('\n') && line.len() < 800, "{line}");
    assert_eq!(bindings_u(), 0);
    // The unresolved-seat channel (Health `unresolved_seats` and its named
    // line, which doctor renders) reports it.
    let summary = m.store.unresolved_seat_summary(&m.budget()).unwrap();
    assert_eq!(summary.count, 3);
    assert!(
        summary.sample.iter().any(|sample| sample.seat == seat_u
            && sample.target.as_ref().map(HostTargetId::as_str) == Some("pane-u")
            && sample.reason == Some(UnresolvedReason::HostInvalidation)),
        "{summary:?}"
    );
    let mut inputs =
        crate::daemon::health::HealthInputs::unknown(uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    inputs.unresolved = Some(summary);
    let health = inputs.assemble();
    assert_eq!(health.unresolved_seats, Some(3));
    assert_eq!(health.validate(), Ok(()));
    assert!(
        health
            .limitations
            .iter()
            .any(|item| item.starts_with("unresolved seats: 3;")
                && item.ends_with("`seat rebind SEAT --pane PANE --operator`")),
        "{:?}",
        health.limitations
    );
    assert!(
        health.limitations.contains(&format!(
            "unresolved seat {} on pane-u (host_invalidation)",
            seat_u.as_str()
        )),
        "{:?}",
        health.limitations
    );
    // Reconnect: the first complete production-shaped snapshot reconfirms
    // every seat, the never-registered one from its structural proof.
    m.reconnect_and_capture(&pair, 1);
    assert_eq!(
        m.seat_states(),
        "resolved:- resolved:- resolved:-",
        "never-registered seat reconfirmed without an operator"
    );
    assert_eq!(m.seat_generation(&seat_u), generation_u);
    assert_eq!(bindings_u(), 0, "reconfirmation registers nothing");
    assert_eq!(
        m.store.unresolved_seat_summary(&m.budget()).unwrap().count,
        0
    );
    // The agent's first fresh lifecycle check-in registers through the
    // shipped CLI: seeded from (and CASed against) the current generation,
    // it registers the seat's next binding generation.
    m.cli(&cli, &["check-in", "--lifecycle-event", "hook-u-1"])
        .unwrap();
    let context = m.cli_context(&cli);
    assert_eq!(context.seat, seat_u);
    assert_eq!(context.binding_generation as i64, generation_u + 1);
    assert_eq!(m.seat_generation(&seat_u), generation_u + 1);
    assert_eq!(
        m.db()
            .query_row(
                "SELECT generation,terminal_id,incarnation,registered_at IS NOT NULL,ended_at IS NULL FROM occupant_bindings WHERE seat_id=?1",
                [seat_u.as_str()],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, bool>(3)?, r.get::<_, bool>(4)?)),
            )
            .unwrap(),
        (
            generation_u + 1,
            Some("term-pane-u".to_string()),
            Some("inc".to_string()),
            true,
            true
        )
    );
    assert_eq!(operator_ops(), operator_before, "no operator repair");
    // Registered now: a further denial reconfirms it through its binding's
    // own evidence, and the same agent registers again by its next hook.
    m.invalidate_host_expecting(
        &pair,
        "unresolved:host_invalidation unresolved:host_invalidation unresolved:host_invalidation",
    );
    m.reconnect_and_capture(&pair, 1);
    assert_eq!(m.seat_states(), "resolved:- resolved:- resolved:-");
    m.cli(&cli, &["check-in", "--lifecycle-event", "hook-u-2"])
        .unwrap();
    assert!(m.cli_context(&cli).binding_generation as i64 > generation_u + 1);
    assert_eq!(operator_ops(), operator_before, "no operator repair");
}

/// Wave-2 fix2 (b) (fix2 review N1): the startup binding-evidence Health
/// state is recomputed when the evidence changes, over the production
/// adapter's shape. A legacy binding the restart cannot backfill keeps its
/// seat unresolved after a denial and Health Degraded with "still lacking
/// 1"; the operator repairs the seat and its agent re-registers (a binding
/// with evidence), and the very next Health read says "still lacking 0" and
/// is no longer Degraded for it, with no daemon restart. The boot-time
/// report stays what the writer found.
/// Kills: Health reading the boot-frozen `binding_evidence_startup` (the
/// line and Degraded state then persist until restart), and the recheck
/// counting a seat whose latest binding now carries evidence.
#[test]
fn startup_evidence_health_recomputes_after_operator_repair_and_reregistration() {
    use crate::ports::{BindingEvidenceStartup, StorePort};
    let mut m = Matrix::new();
    let pair = m.flaky_pair(HostShape::ProductionAdapter);
    m.invalidate_host(&pair);
    m.strip_binding_evidence(Some("pane-s"));
    // The seat's structural proof no longer provably describes this binding
    // (another host epoch), so the restart cannot backfill it.
    m.db()
        .execute(
            "UPDATE occupant_bindings SET host_epoch=host_epoch+100 WHERE target_id='pane-s'",
            [],
        )
        .unwrap();
    m.restart();
    let startup = BindingEvidenceStartup {
        backfilled: 0,
        still_lacking: 1,
    };
    assert_eq!(m.store.binding_evidence_startup(), Some(startup));
    assert_eq!(
        m.store.binding_evidence_current(&m.budget()).unwrap(),
        Some(startup)
    );
    let identity = m.compose_flaky(&pair.host);
    let pair = FlakyPair { identity, ..pair };
    m.reconnect_and_capture(&pair, 1);
    assert_eq!(m.seat_states(), "resolved:- unresolved:host_invalidation");
    let healthy = |report| {
        let mut inputs = crate::daemon::health::HealthInputs::unknown(
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        );
        inputs.database = crate::daemon::health::ComponentStatus::Ready;
        inputs.schema = crate::daemon::health::ComponentStatus::Ready;
        inputs.host = crate::daemon::health::ComponentStatus::Ready;
        inputs.scheduler = crate::daemon::health::ComponentStatus::Ready;
        inputs.current_execution = crate::protocol::results::CapabilityState::Supported;
        inputs.coherent_enumeration = crate::protocol::results::CapabilityState::Supported;
        inputs.safe_prompt = crate::protocol::results::CapabilityState::Supported;
        inputs.receipt_registration = crate::protocol::results::CapabilityState::Supported;
        inputs.codex = crate::daemon::health::HarnessStatus::Supported("native".into());
        inputs.claude = crate::daemon::health::HarnessStatus::Supported("native".into());
        inputs.binding_evidence = report;
        inputs.unresolved = Some(Default::default());
        inputs.assemble()
    };
    let before = healthy(m.store.binding_evidence_current(&m.budget()).unwrap());
    assert_eq!(
        before.state,
        crate::protocol::results::HealthState::Degraded
    );
    assert!(
        before.limitations[0]
            .starts_with("binding evidence: backfilled 0 at store startup, still lacking 1 now;"),
        "{:?}",
        before.limitations
    );
    // Operator repair, then the pane's agent registers with evidence.
    let CommandResult::OperatorRebound(rebound) = m
        .handle(Command::OperatorRebind(
            crate::protocol::commands::OperatorRebind {
                seat: pair.s.seat.clone(),
                target: pair.s.target.clone(),
                operation: OperationId::new("repair-s"),
            },
        ))
        .unwrap()
    else {
        panic!("operator repair")
    };
    assert_eq!(rebound, pair.s.seat);
    let mut next = pair.s.clone();
    next.execution = ExecutionId::new("00000000-0000-4000-8000-0000000000fc");
    m.lifecycle(next, "checkin-s-after-repair");
    let current = m.store.binding_evidence_current(&m.budget()).unwrap();
    assert_eq!(
        current,
        Some(BindingEvidenceStartup {
            backfilled: 0,
            still_lacking: 0,
        })
    );
    // The boot-time report is unchanged; Health uses the recomputed one.
    assert_eq!(m.store.binding_evidence_startup(), Some(startup));
    let after = healthy(current);
    assert_eq!(after.state, crate::protocol::results::HealthState::Healthy);
    assert!(after.limitations.is_empty(), "{:?}", after.limitations);
    assert_eq!(m.seat_states(), "resolved:- resolved:-");
}

/// Failpoints are not part of the request surface: an agent-supplied
/// `failpoint` field anywhere in a wire request is rejected, and an armed
/// failpoint is scoped to one database/instance/boot so it cannot fire for
/// another composed service. (Release absence is proven by
/// tests/service/check_failpoints_absent.sh.)
/// Kills: a request field or a foreign scope that could arm a failpoint.
#[test]
fn failpoints_cannot_be_armed_through_requests_or_foreign_scopes() {
    use crate::protocol::wire::{PROTOCOL_VERSION, WireRequest};
    let m = Matrix::new();
    let (s, _r, thread) = m.pair();
    let base = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "r".into(),
        expected_instance: uuid::Uuid::new_v4().to_string(),
        expected_boot: None,
        output: None,
        command: Command::SendMessage(SendMessage {
            thread: thread.clone(),
            body: "hello".into(),
            invited_recipients: vec![],
            deadline_millis: None,
            operation: OperationId::new("wire"),
            claim: s.clone(),
            relays_user: false,
        }),
    };
    let encoded = serde_json::to_value(&base).unwrap();
    assert!(WireRequest::decode(encoded.to_string().as_bytes()).is_ok());
    for path in [
        &[][..],
        &["command"][..],
        &["command", "args"][..],
        &["command", "args", "claim"][..],
    ] {
        let mut hostile = encoded.clone();
        let mut node = &mut hostile;
        for key in path {
            node = node.get_mut(*key).unwrap();
        }
        node.as_object_mut().unwrap().insert(
            "failpoint".into(),
            serde_json::json!({"name": "send.before_commit", "action": "error"}),
        );
        assert!(
            WireRequest::decode(hostile.to_string().as_bytes()).is_err(),
            "failpoint field accepted at {path:?}"
        );
    }
    // A failpoint armed for another database never fires here.
    let foreign = Failpoint::error(
        "send.before_commit",
        "/nonexistent/other.db",
        ErrorCode::StoreBusy,
    );
    let sent = m.send(&s, &thread, "hello", &[], "send-1").unwrap();
    assert_eq!((foreign.hits(), foreign.fired()), (0, 0));
    assert_eq!(m.pending("r"), vec![sent]);
}
