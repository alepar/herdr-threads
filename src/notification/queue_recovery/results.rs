//! Node-owned exact metadata completions. Capacity is reserved before possible Add.
use super::*;
use crate::ports::{WakeArmTicket, WakeIntentResult};
use std::{
    collections::BTreeMap,
    ops::Bound::{Excluded, Unbounded},
    sync::OnceLock,
};
const CAPACITY: usize = 4096;
struct Entry {
    owner: Uuid,
    ready: OnceLock<(WakeArmTicket, WakeIntentResult)>,
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
        if ticket.attempt != self.attempt || ticket.owner_boot != self.entry.owner {
            return Err(refused("wake result ticket changed"));
        }
        match self.entry.ready.set((ticket, result)) {
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
            let Some((ticket, result)) = entry.ready.get() else {
                continue;
            };
            // No collection mutex is held across writer admission or store I/O.
            let settled = match recovery.write(budget, |store| {
                store.record_wake_intent_result(ticket, result, budget)
            }) {
                Ok(true) => Ok(true),
                Ok(false) => recovery.store.wake_intent(&id, budget).map(|row| {
                    row.is_none_or(|row| {
                        row.owner_boot != ticket.owner_boot
                            || row.arm_nonce != Some(ticket.nonce)
                            || row.phase != WakeIntentPhase::Armed
                    })
                }),
                Err(error) => Err(error),
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
