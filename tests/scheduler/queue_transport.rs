//! Transport selection must not apply composer heuristics to queue effects.
use super::*;
use crate::{
    notification::{
        dispatch::MAX_ACTIVE_PROMPTS,
        queue::{QueueDispatch, QueueWakePort},
    },
    ports::{
        AgentComposerState, NoPokeCapabilities, PokeMode, PokePlan, WakeTarget, WakeTransport,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

#[derive(Default)]
struct QueueClock(AtomicU64);
impl Clock for QueueClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}
struct QueueHost {
    inner: TextRecordingHost,
    guards: Mutex<Vec<u64>>,
    prompts: Mutex<Vec<u64>>,
    verified: AtomicUsize,
    cleared: AtomicUsize,
}
impl QueueHost {
    fn new() -> Self {
        Self {
            inner: TextRecordingHost {
                observation: observation(false),
                host_bound_harness: None,
                foreground: vec![111],
                typed: Mutex::new(Vec::new()),
            },
            guards: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
            verified: AtomicUsize::new(0),
            cleared: AtomicUsize::new(0),
        }
    }
}
impl HostPort for QueueHost {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        self.inner.native_launch_capability()
    }
    fn live_topology(
        &self,
        budget: &CallBudget,
    ) -> Result<crate::host::observation::HostTopology, ApiError> {
        self.inner.live_topology(budget)
    }
    fn observe_current_target_for_archival(
        &self,
        target: &HostTargetId,
        ctx: &HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, ApiError> {
        self.inner.observe_current_target_for_archival(target, ctx)
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        ctx: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.inner.observe_current_target(target, ctx)
    }
    fn foreground_pids(
        &self,
        target: &HostTargetId,
        ctx: &HostCallContext,
    ) -> Result<Vec<u32>, ApiError> {
        self.inner.foreground_pids(target, ctx)
    }
    fn enumerate_targets(&self, ctx: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        self.inner.enumerate_targets(ctx)
    }
    fn safe_wake_target(&self, seat: &SeatId, obs: &HostObservation) -> Option<SafeWakeTarget> {
        self.inner.safe_wake_target(seat, obs)
    }
    fn check_idle_wake(&self, _: &SafeWakeTarget, ctx: &HostCallContext) -> Result<(), ApiError> {
        self.guards.lock().unwrap().push(ctx.budget.deadline.0);
        Ok(())
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        ctx: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.prompts.lock().unwrap().push(ctx.budget.deadline.0);
        self.inner.submit_prompt(target, text, ctx)
    }
    fn submit_prompt_if_owed(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        ctx: &HostCallContext,
        owed: &dyn Fn() -> Result<bool, ApiError>,
    ) -> Result<Option<PromptOutcome>, ApiError> {
        self.check_idle_wake(target, ctx)?;
        if !owed()? {
            return Ok(None);
        }
        self.submit_prompt(target, text, ctx).map(Some)
    }
    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        self.inner.send_submit_key(target, context)
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        ctx: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.inner.launch_native(request, ctx)
    }
    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        self.verified.fetch_add(1, Ordering::SeqCst);
        Ok(AgentComposerState::HoldingPrompt)
    }
    fn clear_own_wake_text(
        &self,
        _: &SafeWakeTarget,
        _: &str,
        _: &HostCallContext,
    ) -> Result<bool, ApiError> {
        self.cleared.fetch_add(1, Ordering::SeqCst);
        Ok(false)
    }
}
struct Queue {
    dispatch: QueueDispatch,
    calls: Mutex<Vec<bool>>,
    clock: Arc<QueueClock>,
    advance: u64,
    panic: bool,
}
impl Queue {
    fn new(dispatch: QueueDispatch, clock: &Arc<QueueClock>) -> Self {
        Self {
            dispatch,
            calls: Mutex::new(Vec::new()),
            clock: clock.clone(),
            advance: 0,
            panic: false,
        }
    }
}
impl QueueWakePort for Queue {
    fn attempt(
        &self,
        _: &WakeReservation,
        _: &SafeWakeTarget,
        use_queue: bool,
        guard: &dyn Fn() -> Result<(), ApiError>,
        owed: &dyn Fn() -> Result<bool, ApiError>,
        _: &HostCallContext,
    ) -> Result<QueueDispatch, ApiError> {
        self.calls.lock().unwrap().push(use_queue);
        self.clock.0.store(self.advance, Ordering::SeqCst);
        guard()?;
        if !owed()? {
            return Ok(QueueDispatch::Refused);
        }
        assert!(!self.panic, "queue effect result lost");
        Ok(self.dispatch)
    }
}
fn hooked() -> WakeReservation {
    let mut r = reservation(ReservedWakeAuthority::Cooperative {
        terminal: TerminalId::new("terminal"),
        incarnation: "incarnation".into(),
        binding_generation: Some(1),
        harness: Some("codex".into()),
        targets: vec![WakeTarget {
            binding: BindingId::new("binding"),
            harness: "codex".into(),
            provenance: "cooperative_top_level".into(),
            fingerprint_pid: Some(111),
            dormant: false,
        }],
    });
    r.reasons = vec!["pending_invitation".into()];
    r
}
fn context() -> HostCallContext {
    HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    }
}

#[test]
fn queue_transport_submitted_unknown_and_refused_never_touch_native_input() {
    for (dispatch, outcome) in [
        (QueueDispatch::Submitted, WakeOutcome::Submitted),
        (QueueDispatch::Unknown, WakeOutcome::OutcomeUnknown),
        (
            QueueDispatch::Refused,
            WakeOutcome::Refused(crate::ports::RefusalCause::Unavailable),
        ),
    ] {
        let clock = Arc::new(QueueClock::default());
        let host = QueueHost::new();
        let queue = Queue::new(dispatch, &clock);
        let dispatcher = NativeWakeDispatcher::new(&host, &CurrentReservation, clock.as_ref())
            .with_queue(&queue);
        let r = hooked();
        assert_eq!(
            dispatcher.attempt_wake(r.clone(), &context()).unwrap(),
            outcome
        );
        assert_eq!(*queue.calls.lock().unwrap(), [true]);
        assert!(host.inner.typed.lock().unwrap().is_empty());
        assert_eq!(host.verified.load(Ordering::SeqCst), 0);
        assert_eq!(host.cleared.load(Ordering::SeqCst), 0);
        assert!(dispatcher.take_verification(&r.seat).is_none());
        assert_eq!(dispatcher.take_transport(&r.attempt), WakeTransport::Queue);
        assert_eq!(
            dispatcher.take_transport(&r.attempt),
            WakeTransport::Composer
        );
    }
}
#[test]
fn queue_transport_guard_uses_original_lease_and_native_fallback_rebudgets() {
    let clock = Arc::new(QueueClock::default());
    let host = QueueHost::new();
    let mut queue = Queue::new(QueueDispatch::Native, &clock);
    queue.advance = 2_500;
    let dispatcher =
        NativeWakeDispatcher::new(&host, &CurrentReservation, clock.as_ref()).with_queue(&queue);
    let r = hooked();
    // HoldingPrompt retains the native verification/clear behavior.
    assert_eq!(
        dispatcher.attempt_wake(r.clone(), &context()).unwrap(),
        WakeOutcome::OutcomeUnknown
    );
    assert_eq!(*host.guards.lock().unwrap(), [5_000, 4_500]);
    assert_eq!(*host.prompts.lock().unwrap(), [4_500]);
    assert_eq!(*host.inner.typed.lock().unwrap(), [WAKE_UP]);
    assert_eq!(host.verified.load(Ordering::SeqCst), 1);
    assert_eq!(host.cleared.load(Ordering::SeqCst), 1);
    assert_eq!(
        dispatcher.take_transport(&r.attempt),
        WakeTransport::Composer
    );
}
#[test]
fn queue_transport_unsafe_modal_is_refused_before_queue() {
    let clock = Arc::new(QueueClock::default());
    let mut host = QueueHost::new();
    host.inner.observation.ui = HostUiState::ApprovalOrQuestion;
    let queue = Queue::new(QueueDispatch::Submitted, &clock);
    let dispatcher =
        NativeWakeDispatcher::new(&host, &CurrentReservation, clock.as_ref()).with_queue(&queue);
    assert_eq!(
        dispatcher.attempt_wake(hooked(), &context()).unwrap(),
        WakeOutcome::Refused(crate::ports::RefusalCause::Unsafe)
    );
    assert!(queue.calls.lock().unwrap().is_empty());
}
#[test]
fn queue_transport_native_only_paths_still_check_durable_fences() {
    for case in 0..5 {
        let clock = Arc::new(QueueClock::default());
        let mut host = QueueHost::new();
        let queue = Queue::new(QueueDispatch::Refused, &clock);
        let mut r = hooked();
        if let ReservedWakeAuthority::Cooperative { targets, .. } = &mut r.authority {
            match case {
                0 => targets[0].provenance = "cooperative_env_session".into(),
                2 => targets[0].dormant = true,
                3 => {
                    targets.push(WakeTarget {
                        binding: BindingId::new("sibling"),
                        ..targets[0].clone()
                    });
                    host.inner.observation.ui = HostUiState::Idle;
                }
                _ => {}
            }
        }
        let outlook = FakeOutlook {
            parked: case == 1,
            only: None,
        };
        let dispatcher = NativeWakeDispatcher::new(&host, &CurrentReservation, clock.as_ref())
            .with_hook_pull(&outlook)
            .with_queue(&queue);
        if case == 4 {
            let plan = PokePlan {
                text: WAKE_UP.into(),
                receipts: Vec::new(),
            };
            dispatcher
                .attempt_poke(
                    r,
                    &plan,
                    PokeMode::WithWake,
                    &NoPokeCapabilities,
                    &context(),
                )
                .unwrap();
        } else {
            dispatcher.attempt_wake(r, &context()).unwrap();
        }
        assert_eq!(*queue.calls.lock().unwrap(), [false], "case {case}");
        assert!(host.inner.typed.lock().unwrap().is_empty());
    }
}
#[test]
fn queue_transport_map_is_bounded_released_by_exact_attempt_and_conservative_on_panic() {
    let clock = Arc::new(QueueClock::default());
    let host = QueueHost::new();
    let queue = Queue::new(QueueDispatch::Submitted, &clock);
    let dispatcher =
        NativeWakeDispatcher::new(&host, &CurrentReservation, clock.as_ref()).with_queue(&queue);
    for index in 0..MAX_ACTIVE_PROMPTS {
        let mut r = hooked();
        r.attempt = WakeAttemptId::new(format!("attempt-{index}"));
        assert_eq!(
            dispatcher.attempt_wake(r, &context()).unwrap(),
            WakeOutcome::Submitted
        );
    }
    let mut fifth = hooked();
    fifth.attempt = WakeAttemptId::new("fifth");
    assert_eq!(
        dispatcher.attempt_wake(fifth.clone(), &context()).unwrap(),
        WakeOutcome::Refused(crate::ports::RefusalCause::Unavailable)
    );
    assert_eq!(queue.calls.lock().unwrap().len(), MAX_ACTIVE_PROMPTS);
    assert_eq!(
        dispatcher.take_transport(&WakeAttemptId::new("unknown")),
        WakeTransport::Composer
    );
    assert_eq!(
        dispatcher.attempt_wake(fifth.clone(), &context()).unwrap(),
        WakeOutcome::Refused(crate::ports::RefusalCause::Unavailable)
    );
    assert_eq!(
        dispatcher.take_transport(&WakeAttemptId::new("attempt-0")),
        WakeTransport::Queue
    );
    assert_eq!(
        dispatcher.attempt_wake(fifth.clone(), &context()).unwrap(),
        WakeOutcome::Submitted
    );
    assert_eq!(
        dispatcher.take_transport(&fifth.attempt),
        WakeTransport::Queue
    );
    let mut panicking = Queue::new(QueueDispatch::Submitted, &clock);
    panicking.panic = true;
    let dispatcher = NativeWakeDispatcher::new(&host, &CurrentReservation, clock.as_ref())
        .with_queue(&panicking);
    let r = hooked();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || dispatcher.attempt_wake(r.clone(), &context())
        ))
        .is_err()
    );
    assert_eq!(dispatcher.take_transport(&r.attempt), WakeTransport::Queue);
    assert_eq!(host.verified.load(Ordering::SeqCst), 0);
}

struct LateOwed(std::sync::atomic::AtomicUsize);
impl ReservationCheck for LateOwed {
    fn is_current(&self, _: &WakeReservation, _: &CallBudget) -> Result<bool, ApiError> {
        Ok(true)
    }
    fn wake_text_evidence_current(
        &self,
        _: &WakeReservation,
        _: &CallBudget,
    ) -> Result<bool, ApiError> {
        Ok(true)
    }
    fn owes_wake(
        &self,
        _: &WakeReservation,
        _: &[Option<&BindingId>],
        _: &CallBudget,
    ) -> Result<bool, ApiError> {
        Ok(self.0.fetch_add(1, Ordering::SeqCst) < 2)
    }
}
#[test]
fn queue_transport_native_fallback_keeps_final_host_owed_check() {
    let clock = Arc::new(QueueClock::default());
    let host = QueueHost::new();
    let queue = Queue::new(QueueDispatch::Native, &clock);
    let check = LateOwed(AtomicUsize::new(0));
    let dispatcher = NativeWakeDispatcher::new(&host, &check, clock.as_ref()).with_queue(&queue);
    assert_eq!(
        dispatcher.attempt_wake(hooked(), &context()).unwrap(),
        WakeOutcome::Refused(crate::ports::RefusalCause::Unavailable)
    );
    assert_eq!(check.0.load(Ordering::SeqCst), 3);
    assert_eq!(*host.guards.lock().unwrap(), [5_000, 2_000]);
    assert!(host.inner.typed.lock().unwrap().is_empty());
}
