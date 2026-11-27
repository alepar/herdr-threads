//! Programmatic wake selection. Message bodies and receipts remain in turn hooks.
use super::queue_recovery::{QueueRecovery, ResultSlot};
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
            if self
                .store
                .queue_wake_selection(r, false, budget)?
                .blockers
                .native
            {
                QueueDispatch::Refused
            } else {
                QueueDispatch::Native
            },
        )
    }
    /// No Add helper was begun, so this invocation definitively did not add.
    fn refuse_without_helper(
        &self,
        r: &WakeReservation,
        slot: &ResultSlot,
    ) -> Result<(), ApiError> {
        let result = WakeIntentResult {
            outcome: WakeIntentOutcome::NoEffect,
            cleanup_confirmed: true,
            diagnostic: "helper not started".into(),
        };
        // Retained until the refusal CAS settles, so a failed write is retried.
        slot.publish_pre_arm(None, None, result.clone())?;
        let budget = self.completion_budget();
        let Some(row) = self.store.wake_intent(&r.attempt, &budget)? else {
            return slot.settled();
        };
        if row.helper.is_none() && row.arm_nonce.is_none() && row.phase == WakeIntentPhase::Prepared
        {
            self.write(&budget, |store| {
                store.refuse_prepared_wake_intent(
                    &r.attempt,
                    row.revision,
                    result.cleanup_confirmed,
                    &result.diagnostic,
                    &budget,
                )
            })?;
        }
        slot.settled()
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
        let Some(result_slot) = self.recovery.reserve_result(r)? else {
            return Ok(QueueDispatch::Refused);
        };
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
                    self.refuse_without_helper(r, &result_slot)?;
                    return self.native_if_clear(r, &context.budget);
                }
                Err(error) => {
                    let _ = self.refuse_without_helper(r, &result_slot);
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
        if !possible_effect {
            // Retained until settled, so a failed refusal metadata write is retried
            // by recovery from this exact completed invocation, never by age.
            result_slot.publish_pre_arm(
                reporting_ticket.clone(),
                Some(journal.op_id()),
                result.clone(),
            )?;
        } else if let Some(ticket) = reporting_ticket.as_ref() {
            result_slot.publish(ticket.clone(), result.clone())?;
        }
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
        if recorded {
            result_slot.settled()?;
        }
        if !recorded && !possible_effect {
            if let Some(current) = self.store.wake_intent(&r.attempt, &budget)?
                && current.phase == WakeIntentPhase::Prepared
                && current.arm_nonce.is_none()
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
            if self
                .store
                .wake_intent(&r.attempt, &budget)?
                .is_none_or(|row| {
                    !matches!(
                        row.phase,
                        WakeIntentPhase::Prepared | WakeIntentPhase::Armed
                    )
                })
            {
                result_slot.settled()?;
            }
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
        let selection = self.store.queue_wake_selection(
            r,
            use_queue && self.programmatic_wake,
            &context.budget,
        )?;
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
                ControlFailure, OwnedWakeEntries, WakeControlTarget, WakeHelperObserver,
                WakeHelperWitness, WakeScope,
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
    struct RetryClock(std::sync::atomic::AtomicU64);
    impl Clock for RetryClock {
        fn utc_now(&self) -> crate::protocol::time::UtcMillis {
            crate::protocol::time::UtcMillis(100 + self.0.load(Ordering::SeqCst) as i64)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1000 + self.0.load(Ordering::SeqCst))
        }
    }
    struct AcceptingControl {
        store: Arc<SqliteStore>,
        reservation: WakeReservation,
        reject_result: bool,
        result_fault: std::sync::Mutex<Option<rusqlite::Connection>>,
        lost_arm_reply: bool,
        clock: Arc<RetryClock>,
        recovery: Arc<QueueRecovery>,
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
                    if self.reject_result {
                        let fault = self.store.context().open_writer().unwrap();
                        fault.execute_batch("CREATE TRIGGER reject_runtime_result BEFORE UPDATE OF phase ON wake_intents WHEN NEW.phase IN ('accepted','no_effect') BEGIN SELECT RAISE(ABORT,'injected accepted result refusal'); END;").unwrap();
                        *self.result_fault.lock().unwrap() = Some(fault);
                    }
                    if self.lost_arm_reply {
                        WakeAddOutcome::RefusedBeforeEffect(
                            unavailable("injected lost arm reply").into(),
                        )
                    } else {
                        self.effects.fetch_add(1, Ordering::SeqCst);
                        WakeAddOutcome::Accepted {
                            id: "queued-runtime-1".into(),
                            cleanup_error: None,
                        }
                    }
                }
                Ok(false) => {
                    WakeAddOutcome::RefusedBeforeEffect(unavailable("final guard refused").into())
                }
                Err(error) => WakeAddOutcome::RefusedBeforeEffect(error.into()),
            };
            observer.completed(true).unwrap();
            // Even positive helper cleanup cannot stand in for a returned result.
            if snapshot().phase == WakeIntentPhase::Armed {
                self.recovery.tick(context.budget).unwrap();
                assert_eq!(snapshot().phase, WakeIntentPhase::Armed);
            }
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
        let clock = Arc::new(RetryClock(std::sync::atomic::AtomicU64::new(0)));
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
            clock.clone(),
            processes,
            state_dir,
            book,
            recovery.clone(),
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
            result_fault: Default::default(),
            lost_arm_reply: false,
            clock,
            recovery,
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
        for offset in [5000, 10000] {
            control.clock.0.store(offset, Ordering::SeqCst);
            assert!(runtime.recovery.tick(&runtime.completion_budget()).is_err());
            assert_eq!(
                control
                    .store
                    .wake_intent(&control.reservation.attempt, &context.budget)
                    .unwrap()
                    .unwrap()
                    .phase,
                WakeIntentPhase::Armed
            );
            assert!(runtime.recovery.has_work());
        }
        control.clock.0.store(15000, Ordering::SeqCst);
        control
            .result_fault
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_runtime_result")
            .unwrap();
        runtime.recovery.tick(&runtime.completion_budget()).unwrap();
        let recovered = control
            .store
            .wake_intent(&control.reservation.attempt, &context.budget)
            .unwrap()
            .unwrap();
        assert_eq!(
            recovered.phase,
            WakeIntentPhase::Accepted,
            "completed result must retry in the same boot"
        );
        assert_eq!(recovered.owned_ids, ["queued-runtime-1"]);
        assert_eq!(
            control.effects.load(Ordering::SeqCst),
            1,
            "metadata retry must not add again"
        );
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
    fn reject_no_effect(store: &SqliteStore) -> rusqlite::Connection {
        let fault = store.context().open_writer().unwrap();
        fault.execute_batch("CREATE TRIGGER reject_runtime_refusal BEFORE UPDATE OF phase ON wake_intents WHEN NEW.phase='no_effect' BEGIN SELECT RAISE(ABORT,'injected refusal metadata failure'); END;").unwrap();
        fault
    }
    fn native_blocked(control: &AcceptingControl, row: &WakeIntentSnapshot) -> bool {
        control
            .store
            .wake_intent_blockers(
                &control.reservation.seat,
                row.preparation.adapter,
                &row.preparation.replay_scope,
                &fixture_support::budget(),
            )
            .unwrap()
            .native
    }
    #[test]
    fn pre_arm_refusal_metadata_failure_retries_same_boot_without_add() {
        let (runtime, control, preparation, key, context) = runtime_fixture(false);
        let fault = std::sync::Mutex::new(None);
        let guard = || {
            *fault.lock().unwrap() = Some(reject_no_effect(&control.store));
            Err(unavailable("UI changed before arm"))
        };
        assert!(
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
                .is_err(),
            "failed refusal metadata surfaces"
        );
        let stranded = control
            .store
            .wake_intent(&control.reservation.attempt, &context.budget)
            .unwrap()
            .unwrap();
        assert_eq!(stranded.phase, WakeIntentPhase::Prepared);
        assert!(stranded.arm_nonce.is_none());
        assert_eq!(stranded.owner_boot, control.reservation.daemon_boot);
        assert!(native_blocked(&control, &stranded));
        control.clock.0.store(5000, Ordering::SeqCst);
        assert!(runtime.recovery.tick(&runtime.completion_budget()).is_err());
        assert_eq!(
            control
                .store
                .wake_intent(&control.reservation.attempt, &context.budget)
                .unwrap()
                .unwrap()
                .phase,
            WakeIntentPhase::Prepared,
            "contention keeps the exact completion retained"
        );
        assert!(runtime.recovery.has_work());
        fault
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_runtime_refusal")
            .unwrap();
        control.clock.0.store(10000, Ordering::SeqCst);
        runtime.recovery.tick(&runtime.completion_budget()).unwrap();
        let row = control
            .store
            .wake_intent(&control.reservation.attempt, &context.budget)
            .unwrap()
            .unwrap();
        assert_eq!(row.phase, WakeIntentPhase::NoEffect);
        assert!(row.arm_nonce.is_none());
        assert!(!row.cleanup_quarantined);
        assert!(row.owned_ids.is_empty());
        assert!(
            !native_blocked(&control, &row),
            "native wake available again"
        );
        assert_eq!(control.calls.load(Ordering::SeqCst), 1, "no Add replay");
        assert_eq!(control.effects.load(Ordering::SeqCst), 0);
        assert!(
            runtime
                .recovery
                .reserve_result(&control.reservation)
                .unwrap()
                .is_some(),
            "settled completion releases its slot"
        );
    }
    #[test]
    fn pre_arm_retry_never_classifies_in_flight_or_other_operations() {
        let (runtime, control, preparation, _key, context) = runtime_fixture(false);
        let PrepareWakeIntent::Prepared(row) = control
            .store
            .prepare_wake_intent(&control.reservation, &preparation, &context.budget)
            .unwrap()
        else {
            panic!("fresh intent prepares")
        };
        let slot = runtime
            .recovery
            .reserve_result(&control.reservation)
            .unwrap()
            .unwrap();
        // In flight: a reserved but unpublished slot is no completion proof.
        control.clock.0.store(5000, Ordering::SeqCst);
        runtime.recovery.tick(&runtime.completion_budget()).unwrap();
        let unchanged = |expected: &WakeIntentSnapshot| {
            assert_eq!(
                &control
                    .store
                    .wake_intent(&control.reservation.attempt, &context.budget)
                    .unwrap()
                    .unwrap(),
                expected
            )
        };
        unchanged(&row);
        let no_effect = WakeIntentResult {
            outcome: WakeIntentOutcome::NoEffect,
            cleanup_confirmed: true,
            diagnostic: "refused".into(),
        };
        assert!(
            slot.publish_pre_arm(
                None,
                None,
                WakeIntentResult {
                    outcome: WakeIntentOutcome::Unknown,
                    ..no_effect.clone()
                }
            )
            .is_err(),
            "only a definitive no-add is a pre-arm completion"
        );
        // A different operation's completion never classifies this row.
        slot.publish_pre_arm(None, Some(uuid::Uuid::new_v4()), no_effect)
            .unwrap();
        drop(slot);
        control.clock.0.store(10000, Ordering::SeqCst);
        runtime.recovery.tick(&runtime.completion_budget()).unwrap();
        unchanged(&row);
        assert!(native_blocked(&control, &row));
        assert!(
            runtime
                .recovery
                .reserve_result(&control.reservation)
                .unwrap()
                .is_some(),
            "inapplicable completion settles without a write"
        );
        assert_eq!(control.calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn pre_arm_retry_keeps_uncertain_started_helper_quarantined() {
        let (runtime, control, preparation, _key, context) = runtime_fixture(false);
        let PrepareWakeIntent::Prepared(row) = control
            .store
            .prepare_wake_intent(&control.reservation, &preparation, &context.budget)
            .unwrap()
        else {
            panic!("fresh intent prepares")
        };
        let slot = runtime
            .recovery
            .reserve_result(&control.reservation)
            .unwrap()
            .unwrap();
        let journal = runtime
            .recovery
            .begin_helper(&row, WakeHelperPurpose::Add, None, &context.budget)
            .unwrap()
            .unwrap();
        let scope = &preparation.target.scope;
        // The probe must answer Unknown on every host: this process's own group is live,
        // but the recorded start never matches its real birth. The fixture scope's pid 10
        // does not exist on macOS, where the probe rightly proves it gone (ht-l8qi).
        journal
            .started(&WakeHelperWitness {
                process: crate::protocol::authority::LivenessFingerprint {
                    pid: std::process::id(),
                    start: scope.process.start,
                },
                kernel_boot: scope.kernel_boot.clone(),
                // SAFETY: getpgrp has no preconditions.
                pgid: unsafe { libc::getpgrp() } as u32,
            })
            .unwrap();
        slot.publish_pre_arm(
            None,
            Some(journal.op_id()),
            WakeIntentResult {
                outcome: WakeIntentOutcome::NoEffect,
                cleanup_confirmed: false,
                diagnostic: "helper cleanup unconfirmed".into(),
            },
        )
        .unwrap();
        drop(slot);
        control.clock.0.store(5000, Ordering::SeqCst);
        runtime.recovery.tick(&runtime.completion_budget()).unwrap();
        let row = control
            .store
            .wake_intent(&control.reservation.attempt, &context.budget)
            .unwrap()
            .unwrap();
        assert_eq!(row.phase, WakeIntentPhase::NoEffect);
        assert!(row.cleanup_quarantined, "uncertain helper stays fenced");
        assert!(!row.native_no_add_exception);
        assert!(native_blocked(&control, &row));
    }
    #[test]
    fn completed_noeffect_after_lost_arm_reply_retries_without_add() {
        let (runtime, mut control, preparation, key, context) = runtime_fixture(true);
        control.lost_arm_reply = true;
        assert!(
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
                .is_err()
        );
        assert_eq!(control.effects.load(Ordering::SeqCst), 0);
        control
            .result_fault
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_runtime_result")
            .unwrap();
        control.clock.0.store(5000, Ordering::SeqCst);
        runtime.recovery.tick(&runtime.completion_budget()).unwrap();
        let row = control
            .store
            .wake_intent(&control.reservation.attempt, &context.budget)
            .unwrap()
            .unwrap();
        assert_eq!(row.phase, WakeIntentPhase::NoEffect);
        assert!(row.arm_nonce.is_some());
        assert!(row.owned_ids.is_empty());
        assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn completed_result_lost_success_reply_and_stale_nonce_settle_without_replay() {
        for stale in [false, true] {
            let (runtime, control, preparation, key, context) = runtime_fixture(false);
            runtime
                .submit_captured(
                    &control.reservation,
                    &key,
                    preparation,
                    &control,
                    &|| Ok(()),
                    &|| Ok(true),
                    &context,
                )
                .unwrap();
            let row = control
                .store
                .wake_intent(&control.reservation.attempt, &context.budget)
                .unwrap()
                .unwrap();
            let slot = runtime
                .recovery
                .reserve_result(&control.reservation)
                .unwrap()
                .unwrap();
            let ticket = WakeArmTicket {
                attempt: row.attempt.clone(),
                owner_boot: row.owner_boot,
                revision: row.revision,
                nonce: if stale {
                    uuid::Uuid::new_v4()
                } else {
                    row.arm_nonce.unwrap()
                },
            };
            let result = WakeIntentResult {
                outcome: WakeIntentOutcome::Accepted {
                    id: "queued-runtime-1".into(),
                },
                cleanup_confirmed: true,
                diagnostic: String::new(),
            };
            slot.publish(ticket.clone(), result.clone()).unwrap();
            slot.publish(ticket.clone(), result.clone()).unwrap();
            let mut other = ticket;
            other.nonce = uuid::Uuid::new_v4();
            assert!(slot.publish(other, result).is_err());
            drop(slot);
            control.clock.0.store(5000, Ordering::SeqCst);
            runtime.recovery.tick(&runtime.completion_budget()).unwrap();
            assert_eq!(
                control
                    .store
                    .wake_intent(&row.attempt, &context.budget)
                    .unwrap()
                    .unwrap(),
                row
            );
            assert!(
                runtime
                    .recovery
                    .reserve_result(&control.reservation)
                    .unwrap()
                    .is_some(),
                "terminal/inapplicable ticket must release only its metadata slot"
            );
            assert_eq!(control.effects.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn result_retry_capacity_is_reserved_before_helper_and_recovered_without_eviction() {
        let (runtime, control, preparation, key, context) = runtime_fixture(false);
        let mut slots = Vec::new();
        for index in 0..4096 {
            let mut r = control.reservation.clone();
            r.attempt = crate::protocol::ids::WakeAttemptId::new(format!("capacity-{index}"));
            slots.push(runtime.recovery.reserve_result(&r).unwrap().unwrap());
        }
        assert_eq!(
            runtime
                .submit_captured(
                    &control.reservation,
                    &key,
                    preparation.clone(),
                    &control,
                    &|| Ok(()),
                    &|| Ok(true),
                    &context
                )
                .unwrap(),
            QueueDispatch::Refused
        );
        assert_eq!(control.calls.load(Ordering::SeqCst), 0);
        assert!(
            control
                .store
                .wake_intent(&control.reservation.attempt, &context.budget)
                .unwrap()
                .is_none()
        );
        let first_ticket = WakeArmTicket {
            attempt: crate::protocol::ids::WakeAttemptId::new("capacity-0"),
            revision: 1,
            owner_boot: control.reservation.daemon_boot,
            nonce: uuid::Uuid::new_v4(),
        };
        slots[0]
            .publish(
                first_ticket.clone(),
                WakeIntentResult {
                    outcome: WakeIntentOutcome::Unknown,
                    cleanup_confirmed: false,
                    diagnostic: String::new(),
                },
            )
            .unwrap();
        drop(slots);
        let mut retained = control.reservation.clone();
        retained.attempt = first_ticket.attempt;
        assert!(
            runtime
                .recovery
                .reserve_result(&retained)
                .unwrap()
                .is_none(),
            "dropping producer must not evict an effectful result"
        );

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
            QueueDispatch::Submitted
        );
        assert_eq!(control.effects.load(Ordering::SeqCst), 1);
    }
}
