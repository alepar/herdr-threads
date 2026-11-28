//! A root and its `/side` session share one harness process: the real dispatcher tries the
//! queue and selection names the root (ht-z41e.37). A dormant target is never mixed with
//! connected ones by `wake_target_rows`, so the dormant case is covered at the gate by
//! `queue_transport_one_process_targets_try_the_queue_and_others_stay_native`.
use super::*;
use crate::protocol::results::ErrorCode;
use crate::{
    harness::adapter::TurnEdge,
    notification::{
        queue::{GuardFor, QueueDispatch, QueueWakePort, QueueWakeRuntime},
        queue_recovery::QueueRecovery,
    },
    service::{
        fair_writer::FairWriter,
        turn_state::{TurnKey, TurnStateBook},
    },
};

const SIDE: &str = "00000000-0000-0000-0000-0000000000a1";

struct MissingProcess {
    boot: String,
}
impl ProcessInfo for MissingProcess {
    fn process(&self, _: u32) -> Option<ProcessRecord> {
        None
    }
    fn kernel_boot(&self) -> Option<String> {
        Some(self.boot.clone())
    }
}
/// Delegates to the real runtime and records each `use_queue` and result.
struct Recording<'a> {
    inner: &'a QueueWakeRuntime,
    seen: Mutex<Vec<(bool, QueueDispatch)>>,
}
impl QueueWakePort for Recording<'_> {
    fn attempt(
        &self,
        r: &WakeReservation,
        target: &SafeWakeTarget,
        use_queue: bool,
        guard: &dyn Fn(GuardFor) -> Result<(), ApiError>,
        owed: &dyn Fn() -> Result<bool, ApiError>,
        ctx: &HostCallContext,
    ) -> Result<QueueDispatch, ApiError> {
        let result = self.inner.attempt(r, target, use_queue, guard, owed, ctx)?;
        self.seen.lock().unwrap().push((use_queue, result));
        Ok(result)
    }
    fn finish_native_fallback(
        &self,
        r: &WakeReservation,
        ticket: uuid::Uuid,
        outcome: NativeFallbackOutcome,
    ) -> Result<(), ApiError> {
        self.inner.finish_native_fallback(r, ticket, outcome)
    }
    fn take_root(&self, attempt: &WakeAttemptId) -> Option<NativeSessionId> {
        self.inner.take_root(attempt)
    }
}

struct Seen {
    outcome: crate::ports::WakeOutcome,
    queue: Vec<(bool, QueueDispatch)>,
    /// Captures of the root's scope (`WakeControl::observe` calls).
    captures: usize,
    /// `authorize` calls, and accepted queue adds.
    authorized: usize,
    effects: usize,
    /// Queue admissions the host was asked, with each `siblings` fact.
    queue_admissions: usize,
    admission_siblings: Vec<bool>,
    /// The provenance fallback line the dispatcher left for the seat.
    fallback: Option<String>,
    pids: Vec<Option<u32>>,
    typed: Vec<String>,
    /// Each target's provenance, for assertion messages.
    provenance: Vec<String>,
}

const SIDE_LIFECYCLE: &[u8] =
    include_bytes!("../fixtures/codex-0.163.0-side/user-prompt-submit-side.json");
const SIDE_TOOL_BOUNDARY: &[u8] =
    include_bytes!("../fixtures/codex-0.163.0-side/pre-tool-use-side.json");

/// Dispatch the fixture seat's wake with a side session enrolled from `payload` in process
/// `side_pid`; returns what the queue port saw, the process captures, the typed-text check and
/// the reservation's target pids.
fn dispatch_with_side(payload: &'static [u8], side_pid: u32) -> Seen {
    dispatch_with_pane(payload, side_pid, Pane::default())
}

/// What the fake pane shows and how its host answers the two admissions.
#[derive(Default)]
struct Pane {
    /// A real read-detection capture: the host's UI and native guard derive from it through the
    /// real Codex composer policy, read with no pane width as the native host does.
    capture: Option<&'static str>,
    /// The pane reads `working` with an empty composer (a sibling's turn), the side's hook `Busy`;
    /// native typing refuses it.
    busy_side: bool,
    /// What the queue admission answers (the native host's answer for the capture).
    queue_refusal: Option<ApiError>,
    /// The seat's `programmatic_wake` is off (the default for a real seat) when the reservation is taken.
    programmatic_off: bool,
}

/// [`dispatch_with_side`] with `pane`.
fn dispatch_with_pane(payload: &'static [u8], side_pid: u32, pane: Pane) -> Seen {
    let off = pane.programmatic_off;
    let (store, _path, r, p) = fixture::fixture_with(|store| {
        fixture::enroll_payload(store, payload, SIDE, side_pid, "side-first-event").unwrap();
        if off {
            fixture::set_programmatic_wake(store, "s", false);
        }
    });
    assert_eq!(
        r.attention_witness.programmatic_wake(),
        !off,
        "the witness carries the store flag"
    );
    let pids: Vec<Option<u32>> = r
        .authority
        .targets()
        .iter()
        .map(|t| t.fingerprint_pid)
        .collect();
    assert_eq!(pids.len(), 2, "the side's items make a second target");
    assert!(r.authority.targets().iter().all(|t| !t.dormant));
    assert!(
        r.authority.targets().iter().all(|t| t.harness == "codex"),
        "both targets are Codex bindings"
    );
    let provenance: Vec<String> = r
        .authority
        .targets()
        .iter()
        .map(|t| t.provenance.to_string())
        .collect();
    assert!(r.reasons.iter().any(|x| x == "pending_invitation"));
    let b = fixture::budget();
    for edge in [TurnEdge::Busy, TurnEdge::Idle] {
        store
            .record_native_turn(
                &fixture::native_report(&store, &r, &p, "qualified", edge),
                &b,
            )
            .unwrap();
    }
    let root = crate::test_support::scratch::temp_path(format!(
        "ht-side-dispatch-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = Arc::new(store);
    let clock = Arc::new(TestClock::default());
    let writer = Arc::new(FairWriter::new(32));
    let processes = Arc::new(MissingProcess {
        boot: p.target.scope.kernel_boot.clone(),
    });
    let book = Arc::new(TurnStateBook::default());
    if pane.busy_side {
        book.record(
            &TurnKey {
                seat: r.seat.clone(),
                generation: p.target.scope.generation,
                harness: p.adapter.as_str().into(),
                native_session: NativeSessionId::new(SIDE),
            },
            TurnEdge::Busy,
            1000,
        );
    }
    book.record(
        &TurnKey {
            seat: r.seat.clone(),
            generation: p.target.scope.generation,
            harness: p.adapter.as_str().into(),
            native_session: p.target.scope.native_session.clone(),
        },
        TurnEdge::Idle,
        1000,
    );
    let recovery = Arc::new(QueueRecovery::new(
        store.clone(),
        writer.clone(),
        clock.clone(),
        processes.clone(),
        root.clone(),
        r.daemon_boot,
    ));
    let arming = Arc::new(fixture::ArmingControl::default());
    let runtime = QueueWakeRuntime::new(
        store.clone(),
        writer,
        clock.clone(),
        processes.clone(),
        root.clone(),
        book,
        recovery,
    )
    .with_wake_control(arming.clone());
    let recording = Recording {
        inner: &runtime,
        seen: Mutex::new(Vec::new()),
    };
    let mut host = QueueHost::new();
    let obs = &mut host.inner.observation;
    obs.target = r.target.clone();
    obs.host_boot = r.host_boot.clone();
    obs.generation = r.target_generation;
    obs.epoch = r.host_epoch;
    obs.terminal = Some(TerminalId::new("term-pane"));
    obs.incarnation = IncarnationEvidence::Verified {
        identity: "inc".into(),
        evidence_kind: EvidenceKind::NativeCurrentTarget,
    };
    obs.ui = HostUiState::Idle;
    if let Some(capture) = pane.capture {
        let read = crate::harness::composer::read_composer(Harness::Codex, capture, None);
        obs.ui = crate::harness::composer::observed_ui(Some("idle"), Some(&read));
        assert_eq!(obs.ui, HostUiState::HumanInput, "{read:?}");
        host.composer = Some(read);
    }
    if pane.busy_side {
        obs.ui = crate::harness::composer::observed_ui(
            Some("working"),
            Some(&crate::harness::composer::ComposerRead::Empty),
        );
        assert_eq!(obs.ui, HostUiState::ActiveTurn);
        host.native_answer = Some(ApiError::new(
            ErrorCode::TargetUnsafe,
            "agent is not awaiting input (status working)",
        ));
    }
    if let Some(refusal) = pane.queue_refusal {
        host.queue_answer = Err(refusal);
    }
    host.inner.foreground = vec![10, side_pid];
    let fence = StoreFence(&store);
    let dispatcher =
        NativeWakeDispatcher::new(&host, &fence, clock.as_ref()).with_queue(&recording);
    let ctx = HostCallContext {
        budget: b.clone(),
        expected_boot: Some(r.host_boot.clone()),
        expected_epoch: Some(r.host_epoch),
    };
    let outcome = dispatcher.attempt_wake(r.clone(), &ctx).unwrap();
    if side_pid == 10 && !off {
        let selected = store.queue_wake_selection(&r, true, &b).unwrap();
        let binding = selected.binding.expect("the root is selected");
        assert_eq!(binding.scope.native_session, p.target.scope.native_session);
        assert!(binding.side_sessions.iter().any(|s| s.as_str() == SIDE));
    }
    let seen = recording.seen.lock().unwrap().clone();
    let fallback = dispatcher.take_text_fallback(&r.seat);
    std::fs::remove_dir_all(root).unwrap();
    let typed: Vec<String> = host
        .inner
        .typed
        .lock()
        .unwrap()
        .iter()
        .map(|t| t.to_string())
        .collect();
    Seen {
        outcome,
        queue: seen,
        captures: arming.calls.load(Ordering::SeqCst),
        authorized: arming.authorized.load(Ordering::SeqCst),
        effects: arming.effects.load(Ordering::SeqCst),
        queue_admissions: host.queue_admissions.load(Ordering::SeqCst),
        admission_siblings: host.admission_siblings.lock().unwrap().clone(),
        fallback,
        pids,
        typed,
        provenance,
    }
}

#[test]
fn side_and_root_in_one_process_queue_through_the_real_dispatcher() {
    // Whatever provenance each enrollment gives the side, the queue wakes the root.
    for (name, payload) in [
        ("user-prompt-submit", SIDE_LIFECYCLE),
        ("pre-tool-use", SIDE_TOOL_BOUNDARY),
    ] {
        let seen = dispatch_with_side(payload, 10);
        let why = format!("{name}: targets {:?}", seen.provenance);
        assert_eq!(seen.pids, [Some(10), Some(10)], "{why}");
        assert_eq!(seen.queue.len(), 1, "{why}");
        assert!(
            seen.queue[0].0,
            "a one-process root and side try the queue: {why}"
        );
        assert_eq!(seen.queue[0].1, QueueDispatch::Submitted, "{why}");
        assert_eq!(seen.outcome, crate::ports::WakeOutcome::Submitted, "{why}");
        assert_eq!(seen.captures, 1, "selection named the root: {why}");
        assert_eq!(seen.effects, 1, "{why}");
        assert!(seen.typed.is_empty(), "nothing is typed: {why}");
        // The side's inbox-marker provenance used to leave a fallback line even though the queue
        // delivered; only a native wake records one.
        assert_eq!(seen.fallback, None, "{why}");
    }
}

#[test]
fn side_in_another_process_stays_native_through_the_real_dispatcher() {
    let seen = dispatch_with_side(SIDE_LIFECYCLE, 11);
    // The dispatcher may refuse before it asks the queue at all.
    assert!(
        seen.queue.iter().all(|(use_queue, _)| !use_queue),
        "targets in two processes never use the queue: {:?}",
        seen.provenance
    );
    assert_eq!(seen.captures, 0);
    assert!(
        seen.typed.is_empty(),
        "nothing is typed into either session"
    );
}

const CODEX_SIDE: &str = include_str!(
    "../../docs/evidence/ansi-composer/captures/codex-0.163.0-side.read-detection.txt"
);
const CODEX_SIDE_DRAFT: &str = include_str!(
    "../../docs/evidence/ansi-composer/captures/codex-0.163.0-side-draft.read-detection.txt"
);

/// Catches the dispatcher refusing a displayed empty `/side` composer's `HumanInput`
/// observation before the queue is asked, or the queue admission refusing its placeholder.
#[test]
fn displayed_side_composer_queue_wakes_the_root_through_the_real_dispatcher() {
    let seen = dispatch_with_pane(
        SIDE_LIFECYCLE,
        10,
        Pane {
            capture: Some(CODEX_SIDE),
            ..Pane::default()
        },
    );
    assert_eq!(
        seen.queue.len(),
        1,
        "the queue was asked: {:?}",
        seen.outcome
    );
    assert!(seen.queue[0].0, "a one-process root and side try the queue");
    assert_eq!(seen.outcome, crate::ports::WakeOutcome::Submitted);
    assert_eq!((seen.captures, seen.effects), (1, 1));
    assert_eq!(seen.queue_admissions, 1, "one host admission per attempt");
    assert_eq!(seen.admission_siblings, [true]);
    assert!(
        seen.typed.is_empty(),
        "native typing refuses the side composer: {:?}",
        seen.typed
    );
    assert_eq!(seen.fallback, None);
}

/// Catches a side draft admitting a queue wake or native typing: the queue admission refuses typed
/// text (what the native host answers for this capture), then the native fallback's own guard
/// refuses the side read.
#[test]
fn displayed_side_draft_refuses_the_queue_and_types_nothing() {
    let seen = dispatch_with_pane(
        SIDE_LIFECYCLE,
        10,
        Pane {
            capture: Some(CODEX_SIDE_DRAFT),
            queue_refusal: Some(ApiError::new(
                ErrorCode::TargetUnsafe,
                "queue wake refused before arm: composer has typed text",
            )),
            ..Pane::default()
        },
    );
    assert_eq!((seen.authorized, seen.effects), (1, 0));
    assert_eq!(seen.queue_admissions, 1);
    assert!(seen.typed.is_empty(), "{:?}", seen.typed);
    assert!(
        matches!(
            seen.outcome,
            crate::ports::WakeOutcome::Refused(crate::ports::RefusalCause::Unsafe)
        ),
        "{:?}",
        seen.outcome
    );
}

/// Catches mapping the queue's admission to native typing's guard: a pane reading `working`
/// because a sibling `/side` turn runs refuses native typing but admits the idle root's queue wake.
#[test]
fn busy_side_working_pane_queue_wakes_the_idle_root_and_native_would_refuse() {
    let seen = dispatch_with_pane(
        SIDE_LIFECYCLE,
        10,
        Pane {
            busy_side: true,
            ..Pane::default()
        },
    );
    assert_eq!(seen.outcome, crate::ports::WakeOutcome::Submitted);
    assert_eq!(seen.effects, 1);
    assert_eq!(seen.admission_siblings, [true]);
    assert!(seen.typed.is_empty(), "{:?}", seen.typed);
}

/// Kills: the queue-candidate exemption applied to a default-off seat (ht-z41e.45): its busy-side
/// pane must be refused by the dispatcher's early check, before any queue selection or capture.
#[test]
fn busy_side_with_programmatic_wake_off_is_refused_before_the_queue_lane() {
    let seen = dispatch_with_pane(
        SIDE_LIFECYCLE,
        10,
        Pane {
            busy_side: true,
            programmatic_off: true,
            ..Pane::default()
        },
    );
    assert!(
        matches!(seen.outcome, crate::ports::WakeOutcome::Refused(_)),
        "{:?}",
        seen.outcome
    );
    assert!(
        seen.queue.is_empty(),
        "the queue port is never asked: {:?}",
        seen.queue
    );
    assert_eq!(
        (
            seen.captures,
            seen.authorized,
            seen.effects,
            seen.queue_admissions
        ),
        (0, 0, 0, 0)
    );
    assert!(seen.typed.is_empty());
}
