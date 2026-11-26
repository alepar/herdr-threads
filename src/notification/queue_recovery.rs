//! Node-wide bounded cleanup of retained wake effects; this grants no wake authority.
use crate::{
    harness::{
        registry::builtins,
        wake::{
            WakeControlContext, WakeHelperObserver, WakeHelperPresence, WakeHelperPurpose,
            WakeHelperQuiescence, WakeHelperWitness,
        },
    },
    ports::{
        ProcessInfo, StorePort, WakeHelperOperation, WakeIntentLogical, WakeIntentPhase,
        WakeIntentSnapshot,
    },
    protocol::{
        ids::WakeAttemptId,
        results::{ApiError, ErrorCode},
        time::{CallBudget, Clock, MonoInstant},
    },
    service::fair_writer::FairWriter,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

mod results;
pub(crate) use results::ResultSlot;
use results::Results;

fn reap_owned_helpers() {
    for registration in builtins().registrations() {
        if let Some(control) = registration.wake_control() {
            control.reap_owned_helpers();
        }
    }
}
fn owned_helpers_pending() -> bool {
    builtins()
        .registrations()
        .iter()
        .filter_map(|registration| registration.wake_control())
        .any(|control| control.has_owned_helpers())
}

fn refused(detail: &str) -> ApiError {
    ApiError::new(ErrorCode::TargetUnsafe, detail)
}
fn accepted(value: bool, detail: &str) -> Result<(), ApiError> {
    if value { Ok(()) } else { Err(refused(detail)) }
}

#[derive(Default)]
struct HelperCompletion {
    observed: Option<bool>,
    committed: bool,
}

/// One committed helper slot. Its callbacks run while the owning daemon lease is retained.
pub struct HelperJournal {
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    clock: Arc<dyn Clock>,
    budget: CallBudget,
    attempt: WakeAttemptId,
    operation: Uuid,
    completion: Mutex<HelperCompletion>,
    work: Arc<AtomicBool>,
}
impl HelperJournal {
    pub fn op_id(&self) -> Uuid {
        self.operation
    }
    /// Positive only after the first physical observation was durably committed.
    pub fn cleanup_confirmed(&self) -> bool {
        self.completion
            .lock()
            .is_ok_and(|completion| completion.committed && completion.observed == Some(true))
    }

    /// Covers both adapter completion and a refusal before the adapter spawned a child.
    pub fn complete(&self, confirmed: bool) -> Result<(), ApiError> {
        let mut completion = self
            .completion
            .lock()
            .map_err(|_| refused("helper journal unavailable"))?;
        // A failed metadata commit must not let an outer error normalizer replace the
        // physical observation. Retries persist that same first observation only.
        let observed = *completion.observed.get_or_insert(confirmed);
        if completion.committed {
            return Ok(());
        }
        // Queue I/O has ended. Preserve its physical cleanup evidence under the held
        // owner lease even when the send budget expired; this budget permits metadata only.
        let metadata = CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0.saturating_add(250)),
            cancellation: Default::default(),
        };
        let _turn = self
            .writer
            .enter_foreground(&metadata, self.clock.as_ref())?;
        accepted(
            self.store.record_wake_helper_completion(
                &self.attempt,
                self.operation,
                observed,
                &metadata,
            )?,
            "helper completion refused",
        )?;
        completion.committed = true;
        Ok(())
    }
}
impl WakeHelperObserver for HelperJournal {
    fn started(&self, witness: &WakeHelperWitness) -> Result<(), ApiError> {
        let _turn = self
            .writer
            .enter_foreground(&self.budget, self.clock.as_ref())?;
        accepted(
            self.store.record_wake_helper_started(
                &self.attempt,
                self.operation,
                witness,
                &self.budget,
            )?,
            "helper witness refused",
        )?;
        self.work.store(true, Ordering::Release);
        Ok(())
    }
    fn completed(&self, confirmed: bool) -> Result<(), ApiError> {
        self.complete(confirmed)
    }
}
#[derive(Default)]
struct Sweep {
    result_after: Option<WakeAttemptId>,
    after: u64,
    last: Option<MonoInstant>,
    work: bool,
}
/// Called by one joined background owner, independently of host wake availability.
pub struct QueueRecovery {
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    clock: Arc<dyn Clock>,
    processes: Arc<dyn ProcessInfo>,
    state_dir: PathBuf,
    boot: Uuid,
    sweep: Mutex<Sweep>,
    results: Arc<Results>,
    work: Arc<AtomicBool>,
}
impl QueueRecovery {
    pub fn new(
        store: Arc<dyn StorePort>,
        writer: Arc<FairWriter>,
        clock: Arc<dyn Clock>,
        processes: Arc<dyn ProcessInfo>,
        state_dir: PathBuf,
        boot: Uuid,
    ) -> Self {
        Self {
            store,
            writer,
            clock,
            processes,
            state_dir,
            boot,
            sweep: Mutex::new(Sweep::default()),
            results: Arc::new(Results::default()),
            work: Arc::new(AtomicBool::new(true)),
        }
    }
    /// Run the one joined node owner. The caller supplies its clock-matched pacer.
    pub fn run(
        &self,
        pacer: &crate::service::pacer::Pacer,
        stop: &crate::protocol::time::Cancellation,
        mut on_error: impl FnMut(&ApiError),
    ) {
        while !stop.is_cancelled() {
            let budget = CallBudget {
                deadline: MonoInstant(self.clock.monotonic_now().0.saturating_add(2500)),
                cancellation: stop.clone(),
            };
            if let Err(error) = self.tick(&budget)
                && !stop.is_cancelled()
            {
                on_error(&error);
            }
            if pacer.wait_blocking(std::time::Duration::from_millis(5000))
                == crate::service::pacer::Wake::Cancelled
            {
                break;
            }
        }
        // Cancellation must not skip this bounded exact-child bookkeeping pass.
        reap_owned_helpers();
    }
    pub fn has_work(&self) -> bool {
        self.work.load(Ordering::Acquire) || self.results.has_work() || owned_helpers_pending()
    }
    pub(crate) fn reserve_result(
        &self,
        reservation: &crate::ports::WakeReservation,
    ) -> Result<Option<ResultSlot>, ApiError> {
        if reservation.daemon_boot != self.boot {
            return Err(refused("wake result owner changed"));
        }
        self.results.reserve(&reservation.attempt, self.boot)
    }
    pub fn begin_helper(
        &self,
        row: &WakeIntentSnapshot,
        purpose: WakeHelperPurpose,
        subject: Option<String>,
        budget: &CallBudget,
    ) -> Result<Option<HelperJournal>, ApiError> {
        let operation = WakeHelperOperation {
            id: Uuid::new_v4(),
            purpose,
            subject,
            owner_boot: self.boot,
            kernel_boot: self
                .processes
                .kernel_boot()
                .filter(|v| !v.is_empty() && v.len() <= 64)
                .ok_or_else(|| refused("helper kernel boot unavailable"))?,
            witness: None,
        };
        let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
        if !self
            .store
            .begin_wake_helper(&row.attempt, row.revision, &operation, budget)?
        {
            return Ok(None);
        }
        Ok(Some(HelperJournal {
            store: self.store.clone(),
            writer: self.writer.clone(),
            clock: self.clock.clone(),
            budget: budget.clone(),
            attempt: row.attempt.clone(),
            operation: operation.id,
            completion: Mutex::new(HelperCompletion::default()),
            work: self.work.clone(),
        }))
    }
    fn write<T>(
        &self,
        budget: &CallBudget,
        call: impl FnOnce(&dyn StorePort) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let _turn = self.writer.enter_background(budget, self.clock.as_ref())?;
        call(self.store.as_ref())
    }
    /// At most 32 summaries and one queue-control invocation in each five-second quantum.
    pub fn tick(&self, budget: &CallBudget) -> Result<(), ApiError> {
        let mut sweep = self
            .sweep
            .lock()
            .map_err(|_| refused("wake recovery unavailable"))?;
        let now = self.clock.monotonic_now();
        if sweep
            .last
            .is_some_and(|last| now.0.saturating_sub(last.0) < 5000)
        {
            return Ok(());
        }
        if budget.is_exhausted(self.clock.as_ref()) {
            return Err(ApiError::new(
                if budget.cancellation.is_cancelled() {
                    ErrorCode::Cancelled
                } else {
                    ErrorCode::DeadlineExceeded
                },
                "wake recovery budget exhausted",
            ));
        }
        sweep.last = Some(now);
        reap_owned_helpers();
        let (result_count, result_error) =
            self.results.retry(self, &mut sweep.result_after, budget)?;
        let page = self.store.wake_intent_recovery_page(
            sweep.after,
            (32 - result_count) as u16,
            budget,
        )?;
        let mut error = result_error;
        let mut next = page.next_after;
        for (index, summary) in page.items.iter().take(32).enumerate() {
            let mut invoked = false;
            match self.visit(&summary.attempt, budget, &mut invoked) {
                Ok(work) => sweep.work |= work,
                Err(failure) => {
                    sweep.work = true;
                    if error.is_none() {
                        error = Some(failure);
                    }
                }
            }
            if invoked {
                next = if index + 1 < page.items.len() || page.next_after.is_some() {
                    Some(summary.ordinal)
                } else {
                    None
                };
                break;
            }
        }
        if let Some(after) = next {
            sweep.after = after;
            // Pagination alone is not activity. Preserve the previous completed
            // sweep (and new helper notifications) until this sweep finishes.
            self.work.fetch_or(sweep.work, Ordering::AcqRel);
        } else {
            sweep.after = 0;
            self.work.store(sweep.work, Ordering::Release);
            sweep.work = false;
        }
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    fn pending(&self, row: &WakeIntentSnapshot) -> bool {
        if row.phase == WakeIntentPhase::Armed {
            return false;
        }
        if row.cleanup_quarantined {
            return row.helper.as_ref().is_some_and(|helper| {
                helper.witness.is_some()
                    || self
                        .processes
                        .kernel_boot()
                        .is_some_and(|boot| boot != helper.kernel_boot)
            });
        }
        row.logical != WakeIntentLogical::Active
            && (!row.cleanup_list_attempted || !row.cleanup_pending_ids.is_empty())
    }
    fn visit(
        &self,
        attempt: &WakeAttemptId,
        budget: &CallBudget,
        invoked: &mut bool,
    ) -> Result<bool, ApiError> {
        let Some(mut row) = self.store.wake_intent(attempt, budget)? else {
            return Ok(false);
        };
        if row.owner_boot != self.boot
            && matches!(
                row.phase,
                WakeIntentPhase::Prepared | WakeIntentPhase::Armed
            )
        {
            self.write(budget, |store| {
                store.recover_wake_intent(attempt, row.revision, row.owner_boot, budget)
            })?;
            let Some(current) = self.store.wake_intent(attempt, budget)? else {
                return Ok(false);
            };
            row = current;
        }
        if self.write(budget, |store| {
            store.retire_wake_intent_if_scope_ended(attempt, budget)
        })? {
            let Some(current) = self.store.wake_intent(attempt, budget)? else {
                return Ok(false);
            };
            row = current;
        }
        if row.logical != WakeIntentLogical::Active
            && row.cleanup_quarantined
            && !row.native_no_add_exception
            && let Some(helper) = row.helper.as_ref().filter(|helper| {
                helper.owner_boot != self.boot
                    && helper.witness.is_none()
                    && matches!(
                        helper.purpose,
                        WakeHelperPurpose::List | WakeHelperPurpose::Delete
                    )
            })
        {
            // Cleanup ownership can differ from the original submission owner. Only the
            // canonical store may grant this native-only exception after lease revocation.
            self.write(budget, |store| {
                store.recover_wake_intent(attempt, row.revision, helper.owner_boot, budget)
            })?;
            let Some(current) = self.store.wake_intent(attempt, budget)? else {
                return Ok(false);
            };
            row = current;
        }
        let control = builtins()
            .by_id(row.preparation.adapter)
            .ok()
            .and_then(|registration| registration.wake_control());
        if row.cleanup_quarantined
            && let Some(helper) = &row.helper
        {
            let presence = if self
                .processes
                .kernel_boot()
                .is_some_and(|boot| !boot.is_empty() && boot != helper.kernel_boot)
            {
                WakeHelperPresence::Quiescent(WakeHelperQuiescence::BootChanged)
            } else if let (Some(witness), Some(control)) = (&helper.witness, control) {
                let context = WakeControlContext {
                    clock: self.clock.as_ref(),
                    budget,
                    processes: self.processes.as_ref(),
                    state_dir: &self.state_dir,
                    helper_observer: None,
                };
                control
                    .probe_helper(witness, &context)
                    .unwrap_or(WakeHelperPresence::Unknown)
            } else {
                WakeHelperPresence::Unknown
            };
            if matches!(presence, WakeHelperPresence::Quiescent(_)) {
                self.write(budget, |store| {
                    store.reconcile_wake_helper(attempt, helper.id, &presence, budget)
                })?;
                let Some(current) = self.store.wake_intent(attempt, budget)? else {
                    return Ok(false);
                };
                row = current;
            }
        }
        // A native Start may fulfill while add is in flight. Only the winning result CAS or
        // old-owner recovery can release Armed for physical cleanup.
        if row.phase == WakeIntentPhase::Armed
            || row.logical == WakeIntentLogical::Active
            || row.cleanup_quarantined
        {
            return Ok(self.pending(&row));
        }
        // A saved clean operation proves that the bounded attempt already ran. Lost progress
        // bookkeeping never replays its List or exact Delete; missing IDs remain best effort.
        if let Some(helper) = &row.helper {
            let progress = match helper.purpose {
                WakeHelperPurpose::List if !row.cleanup_list_attempted => Some((true, None)),
                WakeHelperPurpose::Delete
                    if helper
                        .subject
                        .as_ref()
                        .is_some_and(|id| row.cleanup_pending_ids.contains(id)) =>
                {
                    Some((false, helper.subject.as_deref()))
                }
                _ => None,
            };
            if let Some((listed, id)) = progress {
                accepted(
                    self.write(budget, |store| {
                        store.record_wake_cleanup_progress(attempt, helper.id, listed, id, budget)
                    })?,
                    "saved cleanup progress refused",
                )?;
                let Some(current) = self.store.wake_intent(attempt, budget)? else {
                    return Ok(false);
                };
                row = current;
            }
        }
        let Some(control) = control else {
            return Ok(false);
        };
        let (purpose, subject) = if !row.cleanup_list_attempted {
            (WakeHelperPurpose::List, None)
        } else if let Some(id) = row.cleanup_pending_ids.first() {
            (WakeHelperPurpose::Delete, Some(id.clone()))
        } else {
            return Ok(false);
        };
        let Some(journal) = self.begin_helper(&row, purpose, subject.clone(), budget)? else {
            // Another live cohort operation can defer this finite cleanup pass. Its owner
            // controls liveness; a blocked historical cleanup alone keeps no daemon alive.
            return Ok(false);
        };
        let context = WakeControlContext {
            clock: self.clock.as_ref(),
            budget,
            processes: self.processes.as_ref(),
            state_dir: &self.state_dir,
            helper_observer: Some(&journal),
        };
        *invoked = true;
        let completion = if purpose == WakeHelperPurpose::List {
            let result = control.list(
                &row.preparation.target,
                &row.preparation.correlation,
                &context,
            );
            let completion = journal.complete(
                result
                    .as_ref()
                    .err()
                    .is_none_or(|failure| failure.cleanup_error.is_none()),
            );
            if let Ok(entries) = result {
                let current = self
                    .store
                    .wake_intent(attempt, budget)?
                    .ok_or_else(|| refused("wake cleanup intent absent"))?;
                accepted(
                    self.write(budget, |store| {
                        store.record_owned_wake_ids(
                            attempt,
                            current.revision,
                            &entries.ids,
                            entries.complete,
                            budget,
                        )
                    })?,
                    "owned wake IDs refused",
                )?;
            }
            completion
        } else {
            let result = control.delete(
                &row.preparation.target,
                subject.as_deref().expect("delete subject"),
                &context,
            );
            journal.complete(
                result
                    .as_ref()
                    .err()
                    .is_none_or(|failure| failure.cleanup_error.is_none()),
            )
        };
        // An attempted delete is final for this cleanup pass whether it returned true or false.
        accepted(
            self.write(budget, |store| {
                store.record_wake_cleanup_progress(
                    attempt,
                    journal.op_id(),
                    purpose == WakeHelperPurpose::List,
                    subject.as_deref(),
                    budget,
                )
            })?,
            "wake cleanup progress refused",
        )?;
        completion?;
        let current = self.store.wake_intent(attempt, budget)?;
        Ok(current.as_ref().is_some_and(|row| self.pending(row)))
    }
}
