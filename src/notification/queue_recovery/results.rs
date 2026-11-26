//! Node-owned exact metadata completions. Capacity is reserved before possible Add.
use super::*;
use crate::ports::{WakeArmTicket, WakeIntentOutcome, WakeIntentResult};
use std::{
    collections::BTreeMap,
    ops::Bound::{Excluded, Unbounded},
    sync::OnceLock,
};
const CAPACITY: usize = 4096;
/// A typed terminal result of one exact control invocation, published after it returned.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Completion {
    /// The committed arm winner's result, settled by its exact ticket CAS.
    Armed(WakeArmTicket, WakeIntentResult),
    /// A definitive no-add outcome before any committed arm. `helper` is this
    /// invocation's own Add journal operation (None: refused before beginning one);
    /// `ticket` is a reporting claim whose arm commit may still have won.
    PreArm {
        ticket: Option<WakeArmTicket>,
        helper: Option<Uuid>,
        result: WakeIntentResult,
    },
}
struct Entry {
    owner: Uuid,
    ready: OnceLock<Completion>,
}
#[derive(Default)]
pub(super) struct Results {
    entries: Mutex<BTreeMap<WakeAttemptId, Arc<Entry>>>,
}
pub(crate) struct ResultSlot {
    entries: Arc<Results>,
    attempt: WakeAttemptId,
    entry: Arc<Entry>,
}
impl ResultSlot {
    /// Called only after control.add returns its typed terminal result.
    pub(crate) fn publish(
        &self,
        ticket: WakeArmTicket,
        result: WakeIntentResult,
    ) -> Result<(), ApiError> {
        self.set(Completion::Armed(ticket, result))
    }
    /// Called only after this invocation definitively did not add: control returned
    /// RefusedBeforeEffect, or no Add helper was begun. The retained completion lets
    /// recovery retry the same exact Prepared refusal CAS within this owner boot.
    pub(crate) fn publish_pre_arm(
        &self,
        ticket: Option<WakeArmTicket>,
        helper: Option<Uuid>,
        result: WakeIntentResult,
    ) -> Result<(), ApiError> {
        if result.outcome != WakeIntentOutcome::NoEffect {
            return Err(refused("pre-arm completion must be no-effect"));
        }
        self.set(Completion::PreArm {
            ticket,
            helper,
            result,
        })
    }
    fn set(&self, completion: Completion) -> Result<(), ApiError> {
        let ticket = match &completion {
            Completion::Armed(ticket, _) => Some(ticket),
            Completion::PreArm { ticket, .. } => ticket.as_ref(),
        };
        if ticket.is_some_and(|ticket| {
            ticket.attempt != self.attempt || ticket.owner_boot != self.entry.owner
        }) {
            return Err(refused("wake result ticket changed"));
        }
        match self.entry.ready.set(completion) {
            Ok(()) => Ok(()),
            Err(value) if self.entry.ready.get() == Some(&value) => Ok(()),
            Err(_) => Err(refused("wake result ticket changed")),
        }
    }
    pub(crate) fn settled(&self) -> Result<(), ApiError> {
        self.entries.remove(&self.attempt, &self.entry)
    }
}
impl Drop for ResultSlot {
    fn drop(&mut self) {
        // A typed result outlives this callback until its exact CAS settles.
        if self.entry.ready.get().is_none() {
            let _ = self.settled();
        }
    }
}
impl Results {
    pub(super) fn reserve(
        self: &Arc<Self>,
        attempt: &WakeAttemptId,
        owner: Uuid,
    ) -> Result<Option<ResultSlot>, ApiError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| refused("wake result collection unavailable"))?;
        if entries.len() >= CAPACITY || entries.contains_key(attempt) {
            return Ok(None);
        }
        let entry = Arc::new(Entry {
            owner,
            ready: OnceLock::new(),
        });
        entries.insert(attempt.clone(), entry.clone());
        Ok(Some(ResultSlot {
            entries: self.clone(),
            attempt: attempt.clone(),
            entry,
        }))
    }
    fn remove(&self, attempt: &WakeAttemptId, entry: &Arc<Entry>) -> Result<(), ApiError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| refused("wake result collection unavailable"))?;
        if entries
            .get(attempt)
            .is_some_and(|current| Arc::ptr_eq(current, entry))
        {
            entries.remove(attempt);
        }
        Ok(())
    }
    pub(super) fn has_work(&self) -> bool {
        self.entries
            .lock()
            .map_or(true, |entries| !entries.is_empty())
    }
    pub(super) fn retry(
        &self,
        recovery: &QueueRecovery,
        after: &mut Option<WakeAttemptId>,
        budget: &CallBudget,
    ) -> Result<(usize, Option<ApiError>), ApiError> {
        let page: Vec<_> = {
            let entries = self
                .entries
                .lock()
                .map_err(|_| refused("wake result collection unavailable"))?;
            let page: Vec<_> = match after.as_ref() {
                Some(after) => entries
                    .range((Excluded(after), Unbounded))
                    .take(16)
                    .map(|(id, row)| (id.clone(), row.clone()))
                    .collect(),
                None => entries
                    .iter()
                    .take(16)
                    .map(|(id, row)| (id.clone(), row.clone()))
                    .collect(),
            };
            if page.is_empty() {
                entries
                    .iter()
                    .take(16)
                    .map(|(id, row)| (id.clone(), row.clone()))
                    .collect()
            } else {
                page
            }
        };
        *after = page.last().map(|(id, _)| id.clone());
        let examined = page.len();
        let mut failure = None;
        for (id, entry) in page {
            let Some(completion) = entry.ready.get() else {
                continue;
            };
            // No collection mutex is held across writer admission or store I/O.
            let settled = match completion {
                Completion::Armed(ticket, result) => {
                    recovery.settle_armed(&id, ticket, result, budget)
                }
                Completion::PreArm {
                    ticket,
                    helper,
                    result,
                } => recovery.settle_pre_arm(
                    &id,
                    entry.owner,
                    ticket.as_ref(),
                    *helper,
                    result,
                    budget,
                ),
            };
            match settled {
                Ok(true) => self.remove(&id, &entry)?,
                Ok(false) => {}
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(error)
                    }
                }
            }
        }
        Ok((examined, failure))
    }
}
impl QueueRecovery {
    /// True once the exact armed ticket's result is recorded or no longer applicable.
    fn settle_armed(
        &self,
        id: &WakeAttemptId,
        ticket: &WakeArmTicket,
        result: &WakeIntentResult,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        if self.write(budget, |store| {
            store.record_wake_intent_result(ticket, result, budget)
        })? {
            return Ok(true);
        }
        Ok(self.store.wake_intent(id, budget)?.is_none_or(|row| {
            row.owner_boot != ticket.owner_boot
                || row.arm_nonce != Some(ticket.nonce)
                || row.phase != WakeIntentPhase::Armed
        }))
    }
    /// Retry the exact same-boot Prepared refusal of a completed pre-arm invocation.
    /// Applies only while the row is still that invocation's unarmed Prepared intent
    /// with the same Add helper operation; any other state settles without a write.
    fn settle_pre_arm(
        &self,
        id: &WakeAttemptId,
        owner: Uuid,
        ticket: Option<&WakeArmTicket>,
        helper: Option<Uuid>,
        result: &WakeIntentResult,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        // A reporting claim whose arm commit won is an Armed row: its exact ticket
        // records this definitive no-add result instead.
        if let Some(ticket) = ticket
            && self.write(budget, |store| {
                store.record_wake_intent_result(ticket, result, budget)
            })?
        {
            return Ok(true);
        }
        let Some(row) = self.store.wake_intent(id, budget)? else {
            return Ok(true);
        };
        if row.phase == WakeIntentPhase::Armed
            && ticket.is_some_and(|ticket| {
                row.owner_boot == ticket.owner_boot && row.arm_nonce == Some(ticket.nonce)
            })
        {
            return Ok(false);
        }
        let exact = row.phase == WakeIntentPhase::Prepared
            && row.owner_boot == owner
            && owner == self.boot
            && row.arm_nonce.is_none()
            && row.helper.as_ref().map(|helper| helper.id) == helper;
        if !exact {
            return Ok(true);
        }
        self.write(budget, |store| {
            store.refuse_prepared_wake_intent(
                id,
                row.revision,
                result.cleanup_confirmed,
                &result.diagnostic,
                budget,
            )
        })
    }
}
