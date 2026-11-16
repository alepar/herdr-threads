//! Programmatic wake selection. Message bodies and receipts remain in turn hooks.
use super::queue_recovery::QueueRecovery;
use crate::{
    harness::{
        registry::builtins,
        wake::{WakeAddOutcome, WakeControl, WakeControlContext, WakeHelperPurpose},
    },
    ports::*,
    protocol::{
        results::{ApiError, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::{
        fair_writer::FairWriter,
        turn_state::{TurnKey, TurnState, TurnStateBook},
    },
};
use std::{path::PathBuf, sync::Arc};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueDispatch {
    Native,
    NativeFallback(uuid::Uuid),
    Refused,
    Submitted,
    Unknown,
}
pub trait QueueWakePort: Send + Sync {
    fn finish_native_fallback(
        &self,
        _reservation: &WakeReservation,
        _ticket: uuid::Uuid,
        _outcome: NativeFallbackOutcome,
    ) -> Result<(), ApiError> {
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn attempt(
        &self,
        reservation: &WakeReservation,
        target: &SafeWakeTarget,
        use_queue: bool,
        guard: &dyn Fn() -> Result<(), ApiError>,
        owed: &dyn Fn() -> Result<bool, ApiError>,
        context: &HostCallContext,
    ) -> Result<QueueDispatch, ApiError>;
}
pub struct QueueWakeRuntime {
    programmatic_wake: bool,
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    clock: Arc<dyn Clock>,
    processes: Arc<dyn ProcessInfo>,
    state_dir: PathBuf,
    turn_state: Arc<TurnStateBook>,
    recovery: Arc<QueueRecovery>,
    error_log: Arc<dyn crate::daemon::logs::LaneErrorLog>,
}
fn unavailable(detail: &str) -> ApiError {
    ApiError::new(ErrorCode::TargetUnsafe, detail)
}
impl QueueWakeRuntime {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<dyn StorePort>,
        writer: Arc<FairWriter>,
        clock: Arc<dyn Clock>,
        processes: Arc<dyn ProcessInfo>,
        state_dir: PathBuf,
        turn_state: Arc<TurnStateBook>,
        recovery: Arc<QueueRecovery>,
    ) -> Self {
        Self {
            programmatic_wake: false,
            store,
            writer,
            clock,
            processes,
            state_dir,
            turn_state,
            recovery,
            error_log: Arc::new(crate::daemon::logs::NoopLaneErrorLog),
        }
    }
    pub fn with_programmatic_wake(mut self, enabled: bool) -> Self {
        self.programmatic_wake = enabled;
        self
    }
    pub fn with_error_log(mut self, error_log: Arc<dyn crate::daemon::logs::LaneErrorLog>) -> Self {
        self.error_log = error_log;
        self
    }
    fn write<T>(
        &self,
        budget: &CallBudget,
        call: impl FnOnce(&dyn StorePort) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
        call(self.store.as_ref())
    }
    /// Metadata completion stays bounded after the external-effect budget expires.
    fn completion_budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0.saturating_add(500)),
            cancellation: Cancellation::default(),
        }
    }
    fn native_if_clear(
        &self,
        r: &WakeReservation,
        budget: &CallBudget,
    ) -> Result<QueueDispatch, ApiError> {
        Ok(
            if self.store.queue_wake_selection(r, budget)?.blockers.native {
                QueueDispatch::Refused
            } else {
                QueueDispatch::Native
            },
        )
    }
    fn refuse_without_helper(&self, r: &WakeReservation) -> Result<(), ApiError> {
        let budget = self.completion_budget();
        let Some(row) = self.store.wake_intent(&r.attempt, &budget)? else {
            return Ok(());
        };
        if row.helper.is_none() && row.arm_nonce.is_none() && row.phase == WakeIntentPhase::Prepared
        {
            self.write(&budget, |store| {
                store.refuse_prepared_wake_intent(
                    &r.attempt,
                    row.revision,
                    true,
                    "helper not started",
                    &budget,
                )
            })?;
        }
        Ok(())
    }
    /// Complete an already captured adapter target under canonical submission fences.
    #[allow(clippy::too_many_arguments)]
    fn submit_captured(
        &self,
        r: &WakeReservation,
        key: &TurnKey,
        preparation: WakeIntentPreparation,
        control: &dyn WakeControl,
        guard: &dyn Fn() -> Result<(), ApiError>,
        owed: &dyn Fn() -> Result<bool, ApiError>,
        context: &HostCallContext,
    ) -> Result<QueueDispatch, ApiError> {
        let row = match self.write(&context.budget, |store| {
            store.prepare_wake_intent(r, &preparation, &context.budget)
        })? {
            PrepareWakeIntent::Prepared(row) => row,
            PrepareWakeIntent::Existing(_) => return Ok(QueueDispatch::Refused),
            PrepareWakeIntent::Refused => return self.native_if_clear(r, &context.budget),
        };
        let journal =
            match self
                .recovery
                .begin_helper(&row, WakeHelperPurpose::Add, None, &context.budget)
            {
                Ok(Some(journal)) => journal,
                Ok(None) => {
                    self.refuse_without_helper(r)?;
                    return self.native_if_clear(r, &context.budget);
                }
                Err(error) => {
                    let _ = self.refuse_without_helper(r);
                    return Err(error);
                }
            };
        let wake_context = WakeControlContext {
            clock: self.clock.as_ref(),
            budget: &context.budget,
            processes: self.processes.as_ref(),
            state_dir: &self.state_dir,
            helper_observer: Some(&journal),
        };
        let nonce = uuid::Uuid::new_v4();
        let mut reporting_ticket = None;
        let mut authorize = || {
            if self.turn_state.state(key) != TurnState::Idle {
                return Ok(false);
            }
            guard()?;
            if !owed()? {
                return Ok(false);
            }
            let Some(current) = self.store.wake_intent(&r.attempt, &context.budget)? else {
                return Ok(false);
            };
            // A reporting claim is not add permission. Only the committed arm winner
            // returns true; a lost arm reply still permits reporting definitive no-add.
            reporting_ticket = Some(WakeArmTicket {
                attempt: r.attempt.clone(),
                revision: current.revision.saturating_add(1),
                owner_boot: r.daemon_boot,
                nonce,
            });
            Ok(self
                .write(&context.budget, |store| {
                    store.arm_wake_intent(r, current.revision, nonce, &context.budget)
                })?
                .is_some())
        };
        let outcome = control.add(
            &preparation.target,
            &preparation.correlation,
            &mut authorize,
            &wake_context,
        );
        let (effect, clean, diagnostic) = match &outcome {
            WakeAddOutcome::Accepted { id, cleanup_error } => (
                WakeIntentOutcome::Accepted { id: id.clone() },
                cleanup_error.is_none(),
                cleanup_error
                    .as_ref()
                    .map_or(String::new(), |e| e.detail.clone()),
            ),
            WakeAddOutcome::Unknown(failure) => (
                WakeIntentOutcome::Unknown,
                failure.cleanup_error.is_none(),
                failure.error.detail.clone(),
            ),
            WakeAddOutcome::RefusedBeforeEffect(failure) => (
                WakeIntentOutcome::NoEffect,
                failure.cleanup_error.is_none(),
                failure.error.detail.clone(),
            ),
        };
        // This also closes a slot when validation refused before spawning a child.
        let _ = journal.complete(clean);
        let budget = self.completion_budget();
        let result = WakeIntentResult {
            outcome: effect,
            cleanup_confirmed: journal.cleanup_confirmed(),
            diagnostic: diagnostic.chars().take(512).collect(),
        };
        let possible_effect = !matches!(outcome, WakeAddOutcome::RefusedBeforeEffect(_));
        let recording = match reporting_ticket {
            Some(ticket) => self.write(&budget, |store| {
                store.record_wake_intent_result(&ticket, &result, &budget)
            }),
            None => Ok(false),
        };
        let recorded = match recording {
            Ok(recorded) => recorded,
            Err(error) if possible_effect => {
                let detail: String = error
                    .detail
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(128)
                    .collect();
                self.error_log.record(
                    crate::service::kicks::Lane::Wakes,
                    &ApiError::new(
                        error.code,
                        format!("queue result {}: {detail}", r.attempt.as_str()),
                    ),
                );
                return Ok(QueueDispatch::Unknown);
            }
            Err(error) => return Err(error),
        };
        if !recorded
            && matches!(outcome, WakeAddOutcome::RefusedBeforeEffect(_))
            && let Some(current) = self.store.wake_intent(&r.attempt, &budget)?
        {
            self.write(&budget, |store| {
                store.refuse_prepared_wake_intent(
                    &r.attempt,
                    current.revision,
                    result.cleanup_confirmed,
                    &result.diagnostic,
                    &budget,
                )
            })?;
        }
        match outcome {
            WakeAddOutcome::Accepted { .. } => Ok(if recorded {
                QueueDispatch::Submitted
            } else {
                QueueDispatch::Unknown
            }),
            WakeAddOutcome::Unknown(_) => Ok(QueueDispatch::Unknown),
            WakeAddOutcome::RefusedBeforeEffect(_) => self.native_if_clear(r, &budget),
        }
    }
}
impl QueueWakePort for QueueWakeRuntime {
    fn finish_native_fallback(
        &self,
        reservation: &WakeReservation,
        ticket: uuid::Uuid,
        outcome: NativeFallbackOutcome,
    ) -> Result<(), ApiError> {
        let budget = self.completion_budget();
        self.write(&budget, |store| {
            store.finish_native_wake_fallback(reservation, ticket, outcome, &budget)
        })
        .map(|_| ())
    }
    fn attempt(
        &self,
        r: &WakeReservation,
        target: &SafeWakeTarget,
        use_queue: bool,
        guard: &dyn Fn() -> Result<(), ApiError>,
        owed: &dyn Fn() -> Result<bool, ApiError>,
        context: &HostCallContext,
    ) -> Result<QueueDispatch, ApiError> {
        if target.seat != r.seat
            || target.target != r.target
            || context.expected_boot.as_ref() != Some(&r.host_boot)
            || context.expected_epoch != Some(r.host_epoch)
        {
            return Err(unavailable("queue target changed"));
        }
        // Fences are checked before capture, including unsupported native-only paths.
        let selection = self.store.queue_wake_selection(r, &context.budget)?;
        if selection.blockers.native {
            guard()?;
            if !owed()? {
                return Ok(QueueDispatch::Refused);
            }
            let ticket = uuid::Uuid::new_v4();
            let granted = self.write(&context.budget, |store| {
                store.claim_native_wake_fallback(r, ticket, &context.budget)
            })?;
            return Ok(if granted {
                QueueDispatch::NativeFallback(ticket)
            } else {
                QueueDispatch::Refused
            });
        }
        if !use_queue || !self.programmatic_wake || selection.blockers.programmatic {
            return Ok(QueueDispatch::Native);
        }
        let Some(binding) = selection.binding else {
            return Ok(QueueDispatch::Native);
        };
        let key = TurnKey {
            seat: r.seat.clone(),
            generation: binding.scope.generation,
            harness: binding.adapter.as_str().into(),
            native_session: binding.scope.native_session.clone(),
        };
        match self.turn_state.state(&key) {
            TurnState::Busy => return Ok(QueueDispatch::Refused),
            TurnState::Unknown => return Ok(QueueDispatch::Native),
            TurnState::Idle => {}
        }
        guard()?;
        if !owed()? {
            return Ok(QueueDispatch::Refused);
        }
        let registry = builtins();
        let Some(control) = registry
            .by_id(binding.adapter)
            .ok()
            .and_then(|a| a.wake_control())
        else {
            return Ok(QueueDispatch::Native);
        };
        let observation = WakeControlContext {
            clock: self.clock.as_ref(),
            budget: &context.budget,
            processes: self.processes.as_ref(),
            state_dir: &self.state_dir,
            helper_observer: None,
        };
        let captured = match control.observe(&binding.scope, &observation) {
            Ok(Some(target)) => target,
            _ => return self.native_if_clear(r, &context.budget),
        };
        let preparation = WakeIntentPreparation {
            adapter: binding.adapter,
            replay_scope: binding.replay_scope,
            target: captured,
            correlation: r.attempt.as_str().into(),
        };
        self.submit_captured(r, &key, preparation, control, guard, owed, context)
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use crate::{
        harness::{
            adapter::TurnEdge,
            wake::{
                ControlFailure, OwnedWakeEntries, WakeControlTarget, WakeHelperWitness, WakeScope,
            },
        },
        host::process_table::HostProcessTable,
        store::SqliteStore,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    mod fixture_support {
        use crate as herdr_threads;
        include!("../../tests/fixtures/wake_intent.rs");
    }
    struct AcceptingControl {
        store: Arc<SqliteStore>,
        reservation: WakeReservation,
        reject_result: bool,
        calls: AtomicUsize,
        effects: AtomicUsize,
    }
    impl WakeControl for AcceptingControl {
        fn observe(
            &self,
            _: &WakeScope,
            _: &WakeControlContext<'_>,
        ) -> Result<Option<WakeControlTarget>, ApiError> {
            panic!("captured submission must not observe again")
        }
        fn add(
            &self,
            target: &WakeControlTarget,
            correlation: &str,
            authorize: &mut dyn FnMut() -> Result<bool, ApiError>,
            context: &WakeControlContext<'_>,
        ) -> WakeAddOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let snapshot = || {
                self.store
                    .wake_intent(&self.reservation.attempt, context.budget)
                    .unwrap()
                    .unwrap()
            };
            let prepared = snapshot();
            assert_eq!(prepared.phase, WakeIntentPhase::Prepared);
            assert_eq!(prepared.preparation.target, *target);
            assert_eq!(prepared.preparation.correlation, correlation);
            assert!(prepared.arm_nonce.is_none());
            let helper = prepared.helper.unwrap();
            assert_eq!(helper.purpose, WakeHelperPurpose::Add);
            assert!(helper.witness.is_none());
            let observer = context
                .helper_observer
                .expect("runtime must journal every helper");
            let witness = WakeHelperWitness {
                process: target.scope.process,
                kernel_boot: target.scope.kernel_boot.clone(),
                pgid: target.scope.process.pid,
            };
            observer.started(&witness).unwrap();
            assert_eq!(snapshot().helper.unwrap().witness, Some(witness));
            let outcome = match authorize() {
                Ok(true) => {
                    let armed = snapshot();
                    assert_eq!(armed.phase, WakeIntentPhase::Armed);
                    assert!(armed.arm_nonce.is_some());
                    assert!(armed.arm_baseline.is_some());
                    self.effects.fetch_add(1, Ordering::SeqCst);
                    if self.reject_result {
                        self.store.context().open_writer().unwrap().execute_batch("CREATE TRIGGER reject_runtime_result BEFORE UPDATE OF phase ON wake_intents WHEN NEW.phase='accepted' BEGIN SELECT RAISE(ABORT,'injected accepted result refusal'); END;").unwrap();
                    }
                    WakeAddOutcome::Accepted {
                        id: "queued-runtime-1".into(),
                        cleanup_error: None,
                    }
                }
                Ok(false) => {
                    WakeAddOutcome::RefusedBeforeEffect(unavailable("final guard refused").into())
                }
                Err(error) => WakeAddOutcome::RefusedBeforeEffect(error.into()),
            };
            observer.completed(true).unwrap();
            outcome
        }
        fn list(
            &self,
            _: &WakeControlTarget,
            _: &str,
            _: &WakeControlContext<'_>,
        ) -> Result<OwnedWakeEntries, Box<ControlFailure>> {
            panic!("submission does not perform cleanup list")
        }
        fn delete(
            &self,
            _: &WakeControlTarget,
            _: &str,
            _: &WakeControlContext<'_>,
        ) -> Result<bool, Box<ControlFailure>> {
            panic!("submission does not perform cleanup delete")
        }
    }
    fn runtime_fixture(
        reject_result: bool,
    ) -> (
        QueueWakeRuntime,
        AcceptingControl,
        WakeIntentPreparation,
        TurnKey,
        HostCallContext,
    ) {
        let (store, path, reservation, preparation) = fixture_support::fixture();
        for edge in [TurnEdge::Busy, TurnEdge::Idle] {
            store
                .record_native_turn(
                    &fixture_support::native_report(
                        &store,
                        &reservation,
                        &preparation,
                        "qualified-baseline",
                        edge,
                    ),
                    &fixture_support::budget(),
                )
                .unwrap();
        }
        let store = Arc::new(store);
        let clock: Arc<dyn Clock> = Arc::new(fixture_support::IntentClock);
        let writer = Arc::new(FairWriter::new(32));
        let processes: Arc<dyn ProcessInfo> = Arc::new(HostProcessTable);
        let book = Arc::new(TurnStateBook::default());
        let key = TurnKey {
            seat: reservation.seat.clone(),
            generation: preparation.target.scope.generation,
            harness: preparation.adapter.as_str().into(),
            native_session: preparation.target.scope.native_session.clone(),
        };
        book.record(&key, TurnEdge::Idle, 1000);
        let state_dir = path.with_extension("queue-state");
        let recovery = Arc::new(QueueRecovery::new(
            store.clone(),
            writer.clone(),
            clock.clone(),
            processes.clone(),
            state_dir.clone(),
            reservation.daemon_boot,
        ));
        let runtime = QueueWakeRuntime::new(
            store.clone(),
            writer,
            clock,
            processes,
            state_dir,
            book,
            recovery,
        );
        let context = HostCallContext {
            budget: fixture_support::budget(),
            expected_boot: Some(reservation.host_boot.clone()),
            expected_epoch: Some(reservation.host_epoch),
        };
        let control = AcceptingControl {
            store,
            reservation,
            reject_result,
            calls: AtomicUsize::new(0),
            effects: AtomicUsize::new(0),
        };
        (runtime, control, preparation, key, context)
    }
    #[test]
    fn captured_submission_composes_journal_final_arm_and_known_acceptance() {
        let (runtime, control, preparation, key, context) = runtime_fixture(false);
        let dispatch = runtime
            .submit_captured(
                &control.reservation,
                &key,
                preparation.clone(),
                &control,
                &|| Ok(()),
                &|| Ok(true),
                &context,
            )
            .unwrap();
        assert_eq!(dispatch, QueueDispatch::Submitted);
        let row = control
            .store
            .wake_intent(&control.reservation.attempt, &context.budget)
            .unwrap()
            .unwrap();
        assert_eq!(row.phase, WakeIntentPhase::Accepted);
        assert_eq!(row.owned_ids, ["queued-runtime-1"]);
        assert_eq!(
            row.logical,
            WakeIntentLogical::Active,
            "queue acceptance is not fulfillment"
        );
        assert!(row.helper.unwrap().witness.is_some());
        assert!(!row.cleanup_quarantined);
        assert_eq!(control.effects.load(Ordering::SeqCst), 1);
        assert_eq!(
            runtime
                .submit_captured(
                    &control.reservation,
                    &key,
                    preparation,
                    &control,
                    &|| Ok(()),
                    &|| Ok(true),
                    &context
                )
                .unwrap(),
            QueueDispatch::Refused
        );
        assert_eq!(
            control.calls.load(Ordering::SeqCst),
            1,
            "existing intent cannot replay add"
        );
    }
    #[test]
    fn accepted_effect_with_result_write_failure_is_unknown_and_never_native() {
        #[derive(Default)]
        struct Records(std::sync::Mutex<Vec<String>>);
        impl crate::daemon::logs::LaneErrorLog for Records {
            fn record(&self, _: crate::service::kicks::Lane, error: &ApiError) {
                self.0.lock().unwrap().push(error.detail.clone());
            }
        }
        let (runtime, control, preparation, key, context) = runtime_fixture(true);
        let log = Arc::new(Records::default());
        let runtime = runtime.with_error_log(log.clone());
        assert_eq!(
            runtime
                .submit_captured(
                    &control.reservation,
                    &key,
                    preparation,
                    &control,
                    &|| Ok(()),
                    &|| Ok(true),
                    &context
                )
                .unwrap(),
            QueueDispatch::Unknown
        );
        let diagnostics = log.0.lock().unwrap();
        assert_eq!(
            diagnostics.len(),
            1,
            "possible-effect result failure must be diagnosable"
        );
        assert!(diagnostics[0].contains(control.reservation.attempt.as_str()));
        assert!(diagnostics[0].len() <= 1024);
        assert!(!diagnostics[0].contains('\n'));
        drop(diagnostics);
        let row = control
            .store
            .wake_intent(&control.reservation.attempt, &context.budget)
            .unwrap()
            .unwrap();
        assert_eq!(
            row.phase,
            WakeIntentPhase::Armed,
            "failed result transaction preserves possible effect"
        );
        assert!(row.owned_ids.is_empty());
        assert_eq!(control.effects.load(Ordering::SeqCst), 1);
        let blockers = control
            .store
            .wake_intent_blockers(
                &control.reservation.seat,
                row.preparation.adapter,
                &row.preparation.replay_scope,
                &context.budget,
            )
            .unwrap();
        assert!(blockers.programmatic && blockers.native);
    }
    #[test]
    fn final_guard_refusal_after_prepare_never_arms_or_adds() {
        let (runtime, control, preparation, key, context) = runtime_fixture(false);
        let checks = AtomicUsize::new(0);
        let guard = || {
            let row = control
                .store
                .wake_intent(&control.reservation.attempt, &context.budget)?
                .unwrap();
            assert_eq!(row.phase, WakeIntentPhase::Prepared);
            assert!(row.helper.unwrap().witness.is_some());
            checks.fetch_add(1, Ordering::SeqCst);
            Err(unavailable("UI changed during helper startup"))
        };
        assert_eq!(
            runtime
                .submit_captured(
                    &control.reservation,
                    &key,
                    preparation,
                    &control,
                    &guard,
                    &|| Ok(true),
                    &context
                )
                .unwrap(),
            QueueDispatch::Native
        );
        let row = control
            .store
            .wake_intent(&control.reservation.attempt, &context.budget)
            .unwrap()
            .unwrap();
        assert_eq!(checks.load(Ordering::SeqCst), 1);
        assert_eq!(row.phase, WakeIntentPhase::NoEffect);
        assert!(row.arm_nonce.is_none());
        assert!(row.owned_ids.is_empty());
        assert!(!row.cleanup_quarantined);
        assert_eq!(control.effects.load(Ordering::SeqCst), 0);
    }
}
