//! Soft-deadline poke flow (spec §10) on a real `SqliteStore` with the real
//! effective deadline and a recording fake host. Time is one adjustable clock
//! shared by the store, the dispatcher and the budgets.
//!
//! Layout: seat `s` (cooperatively bound, codex) holds one pending receipt per
//! thread. Every receipt has a frozen window of `WINDOW` ms from time 0, so
//! the frozen soft point is `SOFT` and the frozen deadline is `WINDOW`. A
//! catch-up row on the thread moves both through `receipts::effective_deadline`.
use super::{Scheduler, WakePort};
use crate::{
    harness::{
        composer,
        recipe::{NativeSupport, PokeCapabilities},
    },
    notification::{dispatch::NativeWakeDispatcher, policy::RetryConfig},
    ports::{
        ComposerStash, EvidenceKind, ExecutionEvidence, HostCallContext, HostObservation, HostPort,
        HostSnapshot, HostUiState, IncarnationEvidence, NativeLaunchCapability,
        NativeLaunchOutcome, NativeLaunchRequest, ObservationProvenance, PokeCapabilitySource,
        PromptOutcome, SafeWakeTarget, StructuralOccupancy, WakeTargetBasis,
    },
    protocol::{
        authority::Harness,
        ids::{ExecutionId, HostBootId, HostCallId, HostTargetId, SeatId, TerminalId, ThreadId},
        results::ApiError,
        summary::SummarySettings,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    service::{fair_writer::FairWriter, workers::ScheduledStore},
    store::{SqliteStore, StoreSettings, catch_up, connection::StoreContext},
};
use rusqlite::Connection;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
};

const WINDOW: i64 = 100_000;
/// The frozen soft point: `deadline - ceil(0.4 * window)`.
const SOFT: i64 = 60_000;

struct TestClock(AtomicI64);
impl TestClock {
    fn set(&self, at: i64) {
        self.0.store(at, Ordering::SeqCst);
    }
}
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(u64::try_from(self.0.load(Ordering::SeqCst)).unwrap())
    }
}

/// One host call, in the order the dispatcher made it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Observe,
    Stash,
    Submit(String),
    /// A prompt submitted through the during-turn mode (`poke_during_turn`).
    SubmitDuringTurn(String),
    Restore(String),
}

/// A recording host whose next observation and stash result the test scripts.
struct RecordingHost {
    ui: Mutex<HostUiState>,
    focused: Mutex<bool>,
    stash: Mutex<ComposerStash>,
    log: Mutex<Vec<Call>>,
    /// When set, the observed UI is derived like the native adapter does:
    /// Herdr's agent status plus a composer read of this detection text.
    detection: Mutex<Option<(&'static str, Harness, &'static str)>>,
}
/// The pane width the production-shaped fixtures are read with.
const PANE_WIDTH: Option<u16> = Some(100);
impl RecordingHost {
    fn new(ui: HostUiState, focused: bool) -> Self {
        Self {
            ui: Mutex::new(ui),
            focused: Mutex::new(focused),
            stash: Mutex::new(ComposerStash::Unsupported),
            log: Mutex::new(Vec::new()),
            detection: Mutex::new(None),
        }
    }
    /// A host whose UI comes from `observed_ui` over a captured composer.
    fn production_shaped(status: &'static str, harness: Harness, detection: &'static str) -> Self {
        let host = Self::new(HostUiState::Unknown, false);
        *host.detection.lock().unwrap() = Some((status, harness, detection));
        host
    }
    fn set(&self, ui: HostUiState, focused: bool) {
        *self.ui.lock().unwrap() = ui;
        *self.focused.lock().unwrap() = focused;
    }
    fn log(&self) -> Vec<Call> {
        self.log.lock().unwrap().clone()
    }
    fn clear(&self) {
        self.log.lock().unwrap().clear();
    }
    fn prompts(&self) -> Vec<String> {
        self.log()
            .into_iter()
            .filter_map(|call| match call {
                Call::Submit(text) | Call::SubmitDuringTurn(text) => Some(text),
                _ => None,
            })
            .collect()
    }
}
impl HostPort for RecordingHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.log.lock().unwrap().push(Call::Observe);
        Ok(HostObservation {
            target: target.clone(),
            host_boot: HostBootId::new("host"),
            epoch: 1,
            generation: 1,
            observed_at_utc: UtcMillis(0),
            observed_at_mono: MonoInstant(0),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: match *self.detection.lock().unwrap() {
                Some((status, harness, text)) => composer::observed_ui(
                    Some(status),
                    Some(&composer::read_composer(harness, text, PANE_WIDTH)),
                ),
                None => *self.ui.lock().unwrap(),
            },
            focused: *self.focused.lock().unwrap(),
            terminal: Some(TerminalId::new("term-pane")),
            occupancy: StructuralOccupancy::Occupied,
            incarnation: IncarnationEvidence::Verified {
                identity: "inc".into(),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new("call"),
            connection_epoch: 1,
            observation_sequence: 1,
            started_at_mono: MonoInstant(0),
            completed_at_mono: MonoInstant(0),
        })
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        Ok(())
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        Some(SafeWakeTarget {
            seat: seat.clone(),
            target: observation.target.clone(),
            host_boot: observation.host_boot.clone(),
            generation: observation.generation,
            terminal: TerminalId::new("term-pane"),
            incarnation: "inc".into(),
            basis: WakeTargetBasis::CooperativeAgent,
            epoch: observation.epoch,
            observation_sequence: observation.observation_sequence,
            bound_harness: None,
        })
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        text: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.log.lock().unwrap().push(Call::Submit(text.to_owned()));
        Ok(PromptOutcome::Submitted)
    }
    fn submit_prompt_during_turn(
        &self,
        _: &SafeWakeTarget,
        text: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.log
            .lock()
            .unwrap()
            .push(Call::SubmitDuringTurn(text.to_owned()));
        Ok(PromptOutcome::Submitted)
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn stash_composer(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<ComposerStash, ApiError> {
        self.log.lock().unwrap().push(Call::Stash);
        Ok(self.stash.lock().unwrap().clone())
    }
    fn restore_composer(
        &self,
        _: &SafeWakeTarget,
        saved: &str,
        _: &HostCallContext,
    ) -> Result<(), ApiError> {
        self.log
            .lock()
            .unwrap()
            .push(Call::Restore(saved.to_owned()));
        Ok(())
    }
}

/// Test-only capability evidence, declared for every harness.
struct Declared(PokeCapabilities);
impl PokeCapabilitySource for Declared {
    fn capabilities(&self, _: Harness) -> PokeCapabilities {
        self.0
    }
}
const NO_CAPS: Declared = Declared(PokeCapabilities::NONE);
const STASH_CAPS: Declared = Declared(PokeCapabilities {
    composer_stash: NativeSupport::Supported,
    poke_during_turn: NativeSupport::Unsupported,
});
const DURING_TURN_CAPS: Declared = Declared(PokeCapabilities {
    composer_stash: NativeSupport::Unsupported,
    poke_during_turn: NativeSupport::Supported,
});

struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

struct Fixture {
    clock: Arc<TestClock>,
    /// The real `SqliteStore` behind the production wake/deadline port.
    store: ScheduledStore,
    /// The same store, for starting the production wake worker.
    sqlite: Arc<SqliteStore>,
    path: PathBuf,
    _guard: RemoveOnDrop,
}
impl Fixture {
    /// Seat `s`, cooperatively bound (codex) on pane `pane`, with one pending
    /// legacy receipt per thread in `threads`, each `WINDOW` ms from time 0.
    fn new(threads: &[&str]) -> Self {
        let path = std::env::temp_dir().join(format!("ht-poke-flow-{}.db", uuid::Uuid::new_v4()));
        let clock = Arc::new(TestClock(AtomicI64::new(0)));
        let context = StoreContext::new(path.clone(), clock.clone());
        let guard = RemoveOnDrop(path.clone());
        let db = context.open_writer().unwrap();
        db.execute_batch("\
            INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);\
            INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-pane','inc','native_current_target',1);\
            INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','host',0,'codex','session','self-reported','cooperative_top_level',0,0,'term-pane','inc');\
        ").unwrap();
        for (n, thread) in threads.iter().enumerate() {
            let message = format!("m-{thread}");
            db.execute(
                "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES (?1,'i','topic','goal',0,0,2)",
                [thread],
            )
            .unwrap();
            db.execute(
                "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,'s','joined')",
                [thread],
            )
            .unwrap();
            db.execute(
                "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES (?1,'i',?2,1,'ordinary','body',?3,0)",
                rusqlite::params![message, thread, n as i64 + 1],
            )
            .unwrap();
            db.execute(
                "INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,?2,'s','pending',?3,0,?3)",
                rusqlite::params![message, thread, WINDOW],
            )
            .unwrap();
        }
        drop(db);
        let store = SqliteStore::new(
            context,
            "i",
            StoreSettings {
                daemon_boot: Some(
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                ),
                wake_batch_delay_ms: 0,
                ..StoreSettings::default()
            },
        )
        .unwrap();
        let store = Arc::new(store);
        Self {
            clock,
            store: ScheduledStore::new(store.clone(), Arc::new(FairWriter::new(32))),
            sqlite: store,
            path,
            _guard: guard,
        }
    }

    fn conn(&self) -> Connection {
        StoreContext::new(self.path.clone(), self.clock.clone())
            .open_writer()
            .unwrap()
    }

    /// `soft_poked_at` per thread, sorted by thread id.
    fn poked(&self) -> Vec<(String, Option<i64>)> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT thread_id, soft_poked_at FROM receipts ORDER BY thread_id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// The seat enters catch-up on `thread` at `enter`, then leaves with a
    /// Ready answer at `ready`; the exit grace is `grace` ms, so the effective
    /// deadline becomes `max(frozen, ready + grace)`.
    fn catch_up(&self, thread: &str, enter: i64, ready: i64, grace: u64) {
        let mut conn = self.conn();
        let settings = SummarySettings {
            exit_grace_ms: grace,
            ..SummarySettings::default()
        };
        let seat = SeatId::new("s");
        let thread = ThreadId::new(thread);
        self.clock.set(enter);
        let tx = conn.transaction().unwrap();
        catch_up::enter_or_keep(
            &tx,
            &catch_up::CatchUpEntry {
                seat: &seat,
                thread: &thread,
                frontier_seq: 0,
                binding_generation: 1,
                execution: &ExecutionId::new("self-reported"),
                now: UtcMillis(enter),
            },
            &settings,
        )
        .unwrap();
        tx.commit().unwrap();
        self.clock.set(ready);
        let tx = conn.transaction().unwrap();
        assert!(
            catch_up::on_ready(&tx, &seat, &thread, 1, UtcMillis(ready), &settings).unwrap(),
            "the active row ends on a Ready answer"
        );
        tx.commit().unwrap();
    }

    fn warned(&self) -> Vec<String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT thread_id FROM receipts WHERE warning_message_id IS NOT NULL ORDER BY thread_id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

type Flow<'a, H> =
    Scheduler<'a, ScheduledStore, ScheduledStore, NativeWakeDispatcher<'a, H, ScheduledStore>>;

fn scheduler<'a, H: HostPort>(
    fixture: &'a Fixture,
    dispatcher: &'a NativeWakeDispatcher<'a, H, ScheduledStore>,
    caps: &'a dyn PokeCapabilitySource,
) -> Flow<'a, H> {
    Scheduler::new(
        "i".into(),
        &fixture.store,
        &fixture.store,
        dispatcher,
        RetryConfig::default(),
        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
    )
    .with_poke_capabilities(caps)
}

fn budget(fixture: &Fixture) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(fixture.clock.monotonic_now().0 + 5_000),
        cancellation: Cancellation::default(),
    }
}

/// Advances the clock to `at` and runs one poke scan.
fn tick<H: HostPort, W: WakePort>(
    fixture: &Fixture,
    flow: &Scheduler<'_, ScheduledStore, W, NativeWakeDispatcher<'_, H, ScheduledStore>>,
    at: i64,
) -> (u16, u16) {
    fixture.clock.set(at);
    let outcome = flow.drive_pokes(&budget(fixture)).unwrap();
    (outcome.examined, outcome.attempted)
}

#[test]
fn one_coalesced_poke_after_the_soft_point() {
    let fx = Fixture::new(&["t1", "t2"]);
    let host = RecordingHost::new(HostUiState::Idle, false);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &NO_CAPS);

    // Before the soft point nothing is examined and the host is not touched.
    assert_eq!(tick(&fx, &flow, SOFT - 1), (0, 0));
    assert!(host.log().is_empty());

    assert_eq!(tick(&fx, &flow, SOFT), (1, 1), "one seat, one attempt");
    let prompts = host.prompts();
    assert_eq!(prompts.len(), 1, "two receipts coalesce into one prompt");
    assert!(
        prompts[0].contains("t1") && prompts[0].contains("t2"),
        "the poke names both threads: {}",
        prompts[0]
    );
    assert_eq!(
        host.log().first(),
        Some(&Call::Observe),
        "the host is read before the prompt"
    );
    assert_eq!(
        fx.poked(),
        [("t1".into(), Some(SOFT)), ("t2".into(), Some(SOFT))],
        "host acceptance sets soft_poked_at on both rows"
    );

    // Later ticks, up to the deadline, send nothing.
    host.clear();
    for at in [SOFT + 1, SOFT + 30_000, WINDOW - 1] {
        assert_eq!(tick(&fx, &flow, at), (0, 0), "at {at}");
    }
    assert!(host.log().is_empty());
}

#[test]
fn extension_moves_the_soft_point_later() {
    let fx = Fixture::new(&["t1"]);
    // Entered at 10s, ready at 30s: the exit grace puts the effective
    // deadline at 30s + 120s = 150s, so the soft point moves from 60s to 110s.
    fx.catch_up("t1", 10_000, 30_000, 120_000);
    let new_soft = 150_000 - 40_000;
    let host = RecordingHost::new(HostUiState::Idle, false);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &NO_CAPS);

    // The frozen soft point and the frozen deadline pass without a poke.
    for at in [SOFT, WINDOW - 1, WINDOW, new_soft - 1] {
        assert_eq!(tick(&fx, &flow, at), (0, 0), "at {at}");
    }
    assert!(host.log().is_empty());
    assert_eq!(fx.poked(), [("t1".into(), None)]);

    assert_eq!(tick(&fx, &flow, new_soft), (1, 1));
    assert_eq!(host.prompts().len(), 1);
    assert_eq!(fx.poked(), [("t1".into(), Some(new_soft))]);
}

#[test]
fn poked_receipt_is_not_rearmed_by_a_later_extension() {
    let fx = Fixture::new(&["t1"]);
    let host = RecordingHost::new(HostUiState::Idle, false);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &NO_CAPS);
    assert_eq!(tick(&fx, &flow, SOFT), (1, 1));
    assert_eq!(host.prompts().len(), 1);
    host.clear();

    // After the poke the thread enters catch-up and leaves with a grace that
    // would put a fresh soft point at 175s - 40s = 135s.
    fx.catch_up("t1", 70_000, 75_000, 100_000);
    for at in [80_000, 134_999, 135_000, 174_999] {
        assert_eq!(tick(&fx, &flow, at), (0, 0), "at {at}");
    }
    assert!(host.log().is_empty(), "no second poke: {:?}", host.log());
    assert_eq!(fx.poked(), [("t1".into(), Some(SOFT))], "mark kept");
}

#[test]
fn skips_leave_the_receipt_unpoked_and_the_next_idle_evaluation_pokes() {
    // No capability evidence: ActiveTurn and HumanInput must be skipped. The
    // wake retry spacing fits once between the soft point and the deadline,
    // so each state gets its own fixture: skip, back off, then poke.
    let spacing = i64::try_from(RetryConfig::default().minimum_delay_ms()).unwrap();
    assert!(SOFT + spacing < WINDOW);
    for (ui, focused) in [
        (HostUiState::Idle, true),
        (HostUiState::ApprovalOrQuestion, false),
        (HostUiState::Unknown, false),
        (HostUiState::HumanInput, false),
        (HostUiState::ActiveTurn, false),
    ] {
        let fx = Fixture::new(&["t1"]);
        let host = RecordingHost::new(ui, focused);
        let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
        let flow = scheduler(&fx, &dispatcher, &NO_CAPS);
        assert_eq!(tick(&fx, &flow, SOFT), (1, 1), "{ui:?} focused={focused}");
        assert_eq!(
            host.log(),
            [Call::Observe],
            "{ui:?} focused={focused}: observed, nothing stashed or sent"
        );
        assert_eq!(
            fx.poked(),
            [("t1".into(), None)],
            "{ui:?} focused={focused}: still unpoked"
        );
        // Within the spacing the seat is not even reserved or observed.
        host.clear();
        assert_eq!(tick(&fx, &flow, SOFT + 1), (0, 0), "{ui:?} backs off");
        assert_eq!(tick(&fx, &flow, SOFT + spacing - 1), (0, 0));
        assert!(host.log().is_empty(), "{:?}", host.log());
        // Idle and unfocused: the same receipt is poked once the spacing
        // has elapsed.
        host.set(HostUiState::Idle, false);
        assert_eq!(tick(&fx, &flow, SOFT + spacing), (1, 1));
        assert_eq!(host.prompts().len(), 1);
        assert_eq!(fx.poked(), [("t1".into(), Some(SOFT + spacing))]);
    }
}

#[test]
fn active_turn_pokes_remain_pending_even_with_the_capability() {
    let fx = Fixture::new(&["t1"]);
    let host = RecordingHost::new(HostUiState::ActiveTurn, false);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &DURING_TURN_CAPS);
    assert_eq!(tick(&fx, &flow, SOFT), (1, 1));
    assert_eq!(host.log(), [Call::Observe]);
    assert!(host.prompts().is_empty());
    assert_eq!(fx.poked(), [("t1".into(), None)]);

    // The same receipt is delivered when the host later reports idle.
    host.set(HostUiState::Idle, false);
    let spacing = i64::try_from(RetryConfig::default().minimum_delay_ms()).unwrap();
    assert_eq!(tick(&fx, &flow, SOFT + spacing), (1, 1));
    assert!(matches!(
        host.log().as_slice(),
        [Call::Observe, Call::Observe, Call::Submit(_)]
    ));
    assert_eq!(fx.poked(), [("t1".into(), Some(SOFT + spacing))]);
}

#[test]
fn drafts_never_stash_even_with_test_only_capabilities() {
    // Saved drafts stay untouched, including with historical stash capability.
    let fx = Fixture::new(&["t1"]);
    let host = RecordingHost::new(HostUiState::HumanInput, false);
    *host.stash.lock().unwrap() = ComposerStash::Saved("draft".into());
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &STASH_CAPS);
    assert_eq!(tick(&fx, &flow, SOFT), (1, 1));
    assert_eq!(host.log(), [Call::Observe]);
    assert_eq!(fx.poked(), [("t1".into(), None)]);

    // Failed: no prompt, no restore, nothing marked.
    let fx = Fixture::new(&["t1"]);
    let host = RecordingHost::new(HostUiState::HumanInput, false);
    *host.stash.lock().unwrap() = ComposerStash::Failed("no composer".into());
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &STASH_CAPS);
    assert_eq!(tick(&fx, &flow, SOFT), (1, 1));
    assert_eq!(host.log(), [Call::Observe]);
    assert_eq!(fx.poked(), [("t1".into(), None)]);

    // Without the capability the stash hook is never called.
    let fx = Fixture::new(&["t1"]);
    let host = RecordingHost::new(HostUiState::HumanInput, false);
    *host.stash.lock().unwrap() = ComposerStash::Saved("draft".into());
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &NO_CAPS);
    assert_eq!(tick(&fx, &flow, SOFT), (1, 1));
    assert_eq!(host.log(), [Call::Observe]);
}

#[test]
fn hard_deadline_warning_fires_on_the_effective_deadline_after_skips() {
    let fx = Fixture::new(&["t1"]);
    // Effective deadline 150s (frozen 100s).
    fx.catch_up("t1", 10_000, 30_000, 120_000);
    let host = RecordingHost::new(HostUiState::Idle, true);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &NO_CAPS);
    let due = |at: i64| {
        fx.clock.set(at);
        let out = flow.drive_deadlines(&budget(&fx)).unwrap();
        out.due_warnings_added
    };

    // The focused pane skips the poke at the effective soft point (110s).
    assert_eq!(tick(&fx, &flow, 110_000), (1, 1));
    assert!(host.prompts().is_empty());
    // The frozen deadline passes: the extension holds the warning back.
    assert_eq!(due(WINDOW), 0);
    // The driver ticks every `deadlines::TICK_MILLIS` (5 s), so the last look
    // before the effective deadline is a full tick earlier.
    assert_eq!(due(150_000 - super::deadlines::TICK_MILLIS as i64), 0);
    assert!(fx.warned().is_empty(), "no warning at the frozen deadline");
    assert_eq!(fx.poked(), [("t1".into(), None)]);

    // Once the effective deadline passes the overdue warning is recorded,
    // once, and no poke is sent for a receipt the warning now owns.
    assert_eq!(due(150_000), 1);
    assert_eq!(fx.warned(), ["t1"]);
    assert_eq!(
        due(150_000 + super::deadlines::TICK_MILLIS as i64),
        0,
        "recorded once"
    );
    host.set(HostUiState::Idle, false);
    host.clear();
    assert_eq!(tick(&fx, &flow, 156_000), (0, 0));
    assert!(host.log().is_empty());
}

const CODEX_EMPTY: &str =
    include_str!("../../docs/evidence/poke-spike/captures/codex-q6-workers.read-detection.txt");
const CODEX_DRAFT: &str =
    include_str!("../../docs/evidence/poke-spike/captures/codex-q1-q2-single.read-detection.txt");

/// Production observations carry `agent_status` plus a composer read, never a
/// bare Unknown: an idle codex with an empty composer is poked.
/// Kills: a classification that stays Unknown (every seat churns and none is
/// ever poked, final review F1).
#[test]
fn production_shaped_idle_observation_is_poked() {
    let fx = Fixture::new(&["t1"]);
    let host = RecordingHost::production_shaped("idle", Harness::Codex, CODEX_EMPTY);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &NO_CAPS);
    assert_eq!(tick(&fx, &flow, SOFT), (1, 1));
    assert!(
        matches!(host.log().as_slice(), [Call::Observe, Call::Submit(text)] if text.contains("t1")),
        "{:?}",
        host.log()
    );
    assert_eq!(fx.poked(), [("t1".into(), Some(SOFT))]);
}

/// Typed input in an idle pane is HumanInput and always deferred.
#[test]
fn production_shaped_draft_observation_always_defers_with_the_capability() {
    let fx = Fixture::new(&["t1"]);
    let host = RecordingHost::production_shaped("idle", Harness::Codex, CODEX_DRAFT);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &NO_CAPS);
    assert_eq!(tick(&fx, &flow, SOFT), (1, 1));
    assert_eq!(host.log(), [Call::Observe], "skipped: typed input");
    assert_eq!(fx.poked(), [("t1".into(), None)]);

    let fx = Fixture::new(&["t1"]);
    let host = RecordingHost::production_shaped("idle", Harness::Codex, CODEX_DRAFT);
    *host.stash.lock().unwrap() = ComposerStash::Saved("hello world one".into());
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &STASH_CAPS);
    assert_eq!(tick(&fx, &flow, SOFT), (1, 1));
    assert_eq!(host.log(), [Call::Observe]);
    assert_eq!(fx.poked(), [("t1".into(), None)]);
}

/// An Unknown observation is skipped, and the seat is then left alone until
/// the wake retry spacing has elapsed: no reservation and no host read on the
/// ~100 ms ticks in between (final review F1).
/// Kills: re-reserving and re-observing a skipped seat on every tick.
#[test]
fn unknown_observation_backs_off() {
    let fx = Fixture::new(&["t1"]);
    let host = RecordingHost::new(HostUiState::Unknown, false);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let flow = scheduler(&fx, &dispatcher, &NO_CAPS);
    let spacing = i64::try_from(RetryConfig::default().minimum_delay_ms()).unwrap();
    let observes = |host: &RecordingHost| {
        host.log()
            .iter()
            .filter(|call| **call == Call::Observe)
            .count()
    };
    assert_eq!(tick(&fx, &flow, SOFT), (1, 1));
    assert_eq!(observes(&host), 1);
    let mut at = SOFT;
    while at + 100 < SOFT + spacing {
        at += 100;
        assert_eq!(tick(&fx, &flow, at), (0, 0), "at {at}");
    }
    assert_eq!(observes(&host), 1, "no read inside the spacing");
    assert_eq!(tick(&fx, &flow, SOFT + spacing), (1, 1));
    assert_eq!(observes(&host), 2, "exactly one more after the spacing");
    assert!(host.prompts().is_empty());
}

/// The store as the wake port, counting `reserve_poke` calls.
struct CountingPokes<'a> {
    store: &'a ScheduledStore,
    reserves: AtomicUsize,
}
impl<'a> CountingPokes<'a> {
    fn new(store: &'a ScheduledStore) -> Self {
        Self {
            store,
            reserves: AtomicUsize::new(0),
        }
    }
    fn reserves(&self) -> usize {
        self.reserves.load(Ordering::SeqCst)
    }
}
impl WakePort for CountingPokes<'_> {
    fn clock(&self) -> &dyn Clock {
        WakePort::clock(self.store)
    }
    fn wake_candidates(
        &self,
        page: crate::protocol::pagination::PageRequest,
        budget: &CallBudget,
    ) -> Result<crate::protocol::pagination::Page<crate::ports::WakeCandidate>, ApiError> {
        WakePort::wake_candidates(self.store, page, budget)
    }
    fn reserve_wake(
        &self,
        candidate: &crate::ports::WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<crate::ports::WakeReservation>, ApiError> {
        WakePort::reserve_wake(self.store, candidate, budget)
    }
    fn complete_wake(
        &self,
        attempt: crate::protocol::ids::WakeAttemptId,
        outcome: crate::ports::WakeOutcome,
        refused_restore: Option<&crate::ports::PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        WakePort::complete_wake(self.store, attempt, outcome, refused_restore, budget)
    }
    fn wake_recovery_candidates(
        &self,
        page: crate::protocol::pagination::PageRequest,
        budget: &CallBudget,
    ) -> Result<crate::protocol::pagination::Page<crate::ports::WakeRecoveryCandidate>, ApiError>
    {
        WakePort::wake_recovery_candidates(self.store, page, budget)
    }
    fn recover_wake_reservation(
        &self,
        request: crate::ports::WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<crate::ports::WakeRecoveryOutcome, ApiError> {
        WakePort::recover_wake_reservation(self.store, request, budget)
    }
    fn poke_candidates(
        &self,
        limit: u16,
        budget: &CallBudget,
    ) -> Result<Vec<crate::ports::PokeDue>, ApiError> {
        WakePort::poke_candidates(self.store, limit, budget)
    }
    fn poke_for_wake(
        &self,
        seat: &SeatId,
        budget: &CallBudget,
    ) -> Result<Option<crate::ports::PokeDue>, ApiError> {
        WakePort::poke_for_wake(self.store, seat, budget)
    }
    fn reserve_poke(
        &self,
        due: &crate::ports::PokeDue,
        budget: &CallBudget,
    ) -> Result<Option<crate::ports::PokeReservation>, ApiError> {
        self.reserves.fetch_add(1, Ordering::SeqCst);
        WakePort::reserve_poke(self.store, due, budget)
    }
    fn complete_poke(
        &self,
        attempt: crate::protocol::ids::WakeAttemptId,
        outcome: crate::ports::WakeOutcome,
        receipts: &[crate::ports::PokeReceipt],
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        WakePort::complete_poke(self.store, attempt, outcome, receipts, budget)
    }
}

type CountingFlow<'a, H> =
    Scheduler<'a, ScheduledStore, CountingPokes<'a>, NativeWakeDispatcher<'a, H, ScheduledStore>>;

fn counting_scheduler<'a, H: HostPort>(
    fixture: &'a Fixture,
    port: &'a CountingPokes<'a>,
    dispatcher: &'a NativeWakeDispatcher<'a, H, ScheduledStore>,
) -> CountingFlow<'a, H> {
    Scheduler::new(
        "i".into(),
        &fixture.store,
        port,
        dispatcher,
        RetryConfig::default(),
        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
    )
    .with_poke_capabilities(&NO_CAPS)
}

/// A reservation the store refuses (the observation's epoch no longer matches
/// the host's) is backed off like a skipped attempt: one `reserve_poke` per
/// retry spacing, not one per ~100 ms tick.
/// Kills: recording no backoff when `reserve_poke` returns `None`.
#[test]
fn refused_reservation_backs_off() {
    let fx = Fixture::new(&["t1"]);
    fx.conn()
        .execute("UPDATE observed_targets SET epoch=2", [])
        .unwrap();
    let host = RecordingHost::new(HostUiState::Idle, false);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let port = CountingPokes::new(&fx.store);
    let flow = counting_scheduler(&fx, &port, &dispatcher);
    let spacing = i64::try_from(RetryConfig::default().minimum_delay_ms()).unwrap();
    assert!(SOFT + spacing < WINDOW);

    assert_eq!(tick(&fx, &flow, SOFT), (1, 0), "refused: not attempted");
    assert_eq!(port.reserves(), 1);
    let mut at = SOFT;
    while at + 100 < SOFT + spacing {
        at += 100;
        assert_eq!(tick(&fx, &flow, at), (0, 0), "at {at}");
    }
    assert_eq!(port.reserves(), 1, "no reservation inside the spacing");
    assert_eq!(tick(&fx, &flow, SOFT + spacing), (1, 0));
    assert_eq!(port.reserves(), 2, "exactly one more after the spacing");
    assert!(host.log().is_empty());
    assert_eq!(fx.poked(), [("t1".into(), None)]);
}

/// A person's seat is never a candidate: no examination, no reservation.
/// Kills: collecting seats `current_authority` can never reserve.
#[test]
fn human_bound_seat_costs_no_reservation() {
    let fx = Fixture::new(&["t1"]);
    fx.conn()
        .execute("UPDATE occupant_bindings SET harness='human'", [])
        .unwrap();
    let host = RecordingHost::new(HostUiState::Idle, false);
    let dispatcher = NativeWakeDispatcher::new(&host, &fx.store, fx.clock.as_ref());
    let port = CountingPokes::new(&fx.store);
    let flow = counting_scheduler(&fx, &port, &dispatcher);
    for at in [SOFT, SOFT + 100, SOFT + 200] {
        assert_eq!(tick(&fx, &flow, at), (0, 0), "at {at}");
    }
    assert_eq!(port.reserves(), 0);
    assert_eq!(fx.poked(), [("t1".into(), None)]);
    assert!(host.prompts().is_empty());
    assert!(host.log().is_empty());
}

/// While Herdr is down the wake lane is frozen (ht-72q), and the frozen pass
/// also skips the soft-deadline poke drive: a seat past its soft point is
/// neither read nor poked until the observation lane's recovery kick, which
/// alone (no clock movement) makes the production wake worker poke it.
/// Kills: a freeze that gates the wake drive but still runs `drive_pokes`.
#[test]
fn frozen_wake_lane_neither_reads_nor_pokes_until_recovery() {
    use crate::service::{
        host_reachability::HostReachability,
        pacer::Pacer,
        workers::{WorkerStatus, start_wake_worker},
    };
    use std::time::{Duration, Instant};
    struct Stop(Cancellation, Option<std::thread::JoinHandle<()>>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.cancel();
            if let Some(worker) = self.1.take() {
                let _ = worker.join();
            }
        }
    }
    let fx = Fixture::new(&["t1"]);
    fx.clock.set(SOFT);
    let host = Arc::new(RecordingHost::new(HostUiState::Idle, false));
    let cancel = Cancellation::default();
    let pacer = Arc::new(Pacer::new("wake", fx.clock.clone(), cancel.clone()));
    let reachability = Arc::new(HostReachability::default());
    reachability.attach_wake_pacer(pacer.clone());
    reachability.mark_down();
    let worker = start_wake_worker(
        fx.sqlite.clone(),
        Arc::new(FairWriter::new(32)),
        host.clone(),
        "i".into(),
        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        RetryConfig::default(),
        pacer.clone(),
        cancel.clone(),
        Arc::new(WorkerStatus::default()),
        Arc::new(Declared(PokeCapabilities::NONE)),
        reachability.clone(),
        Arc::new(crate::ports::NoModChannels),
    )
    .unwrap();
    let _stop = Stop(cancel, Some(worker));
    let until = Instant::now() + Duration::from_secs(5);
    while pacer.idle_events() < 1 {
        assert!(Instant::now() < until, "the frozen lane never blocked");
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(60));
    assert!(host.log().is_empty(), "a frozen lane read the host");
    assert_eq!(fx.poked(), [("t1".into(), None)], "a frozen lane poked");
    reachability.mark_up();
    let until = Instant::now() + Duration::from_secs(5);
    while fx.poked() != [("t1".into(), Some(SOFT))] {
        assert!(Instant::now() < until, "no poke after the recovery kick");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(host.prompts().len(), 1, "one poke after recovery");
}
